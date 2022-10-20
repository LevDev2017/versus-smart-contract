//! # A Concordium V1 smart contract
use concordium_std::*;
use core::fmt::Debug;

/// Tag for the NewAdmin event. The CIS-2 library already uses the
/// event tags from `u8::MAX` to `u8::MAX - 4`.
pub const TOKEN_NEW_ADMIN_EVENT_TAG: u8 = u8::MAX - 5;

/// Tag for the NewImplementation event.
pub const TOKEN_NEW_IMPLEMENTATION_EVENT_TAG: u8 = u8::MAX - 6;

// Types

/// This parameter is used as the return value of the fallback function.
#[derive(PartialEq, Eq, Debug)]
struct RawReturnValue(Vec<u8>);

impl Serial for RawReturnValue {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> { out.write_all(&self.0) }
}

/// Tagged events to be serialized for the event log.
enum VersusEvent {
    /// A new admin event.
    NewAdmin(NewAdminEvent),
    /// A new implementation event.
    NewImplementation(NewImplementationEvent),
}

impl Serial for VersusEvent {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        match self {
            VersusEvent::NewAdmin(event) => {
                out.write_u8(TOKEN_NEW_ADMIN_EVENT_TAG)?;
                event.serial(out)
            }
            VersusEvent::NewImplementation(event) => {
                out.write_u8(TOKEN_NEW_IMPLEMENTATION_EVENT_TAG)?;
                event.serial(out)
            }
        }
    }
}

/// The `proxy` contract state.
#[derive(Serial, Deserial, Clone, SchemaType)]
struct StateProxy {
    /// The admin address can upgrade the implementation contract.
    admin:                  Address,
    /// Address of the w_ccd implementation contract.
    implementation_address: ContractAddress,
    /// Address of the w_ccd state contract.
    state_address:          ContractAddress,
}

/// NewAdminEvent.
#[derive(Serial)]
struct NewAdminEvent {
    /// New admin address.
    new_admin: Address,
}

/// NewImplementationEvent.
#[derive(Serial)]
struct NewImplementationEvent {
    /// New implementation address.
    new_implementation: ContractAddress,
}

/// The parameter type for the state contract function `initialize`.
#[derive(Serialize, SchemaType)]
struct InitializeStateParams {
    /// Address of the w_ccd proxy contract.
    proxy_address:          ContractAddress,
    /// Address of the w_ccd implementation contract.
    implementation_address: ContractAddress,
}

/// The parameter type for the implementation contract function `initialize`.
#[derive(Serialize, SchemaType)]
struct InitializeImplementationParams {
    /// Address of the w_ccd proxy contract.
    proxy_address: ContractAddress,
    /// Address of the w_ccd state contract.
    state_address: ContractAddress,
}

/// The parameter type for the proxy contract function `init`.
#[derive(Serialize, SchemaType)]
struct InitProxyParams {
    /// Address of the w_ccd implementation contract.
    implementation_address: ContractAddress,
    /// Address of the w_ccd state contract.
    state_address:          ContractAddress,
}

/// The parameter type for the state contract function
/// `set_implementation_address`.
#[derive(Serialize, SchemaType)]
struct SetImplementationAddressParams {
    /// Address of the w_ccd implementation contract.
    implementation_address: ContractAddress,
}

/// The different errors the contract can produce.
#[derive(Serialize, Debug, PartialEq, Eq, Reject, SchemaType)]
enum CustomContractError {
    /// Failed parsing the parameter.
    #[from(ParseError)]
    ParseParams,
    /// Failed logging: Log is full.
    LogFull,
    /// Failed logging: Log is malformed.
    LogMalformed,
    /// Failed to invoke a contract.
    InvokeContractError,
    /// Failed to invoke a transfer.
    InvokeTransferError,
    /// Contract is paused.
    ContractPaused,
    /// Contract already initialized.
    AlreadyInitialized,
    /// Contract not initialized.
    UnInitialized,
    /// Only implementation contract.
    OnlyImplementation,
    /// Only proxy contract.
    OnlyProxy,
    /// Raised when implementation/proxy can not invoke state contract.
    StateInvokeError,
    /// Only admin
    OnlyAdmin,
}

type ContractResult<A> = Result<A, CustomContractError>;

/// Mapping the logging errors to ContractError.
impl From<LogError> for CustomContractError {
    fn from(le: LogError) -> Self {
        match le {
            LogError::Full => Self::LogFull,
            LogError::Malformed => Self::LogMalformed,
        }
    }
}

/// Mapping errors related to contract invocations to CustomContractError.
impl<T> From<CallContractError<T>> for CustomContractError {
    fn from(_cce: CallContractError<T>) -> Self { Self::InvokeContractError }
}

/// This function logs an event.
#[receive(
    contract = "Versus-Proxy",
    name = "logEvent",
    error = "CustomContractError",
    enable_logger
)]
fn contract_proxy_log_event<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &impl HasHost<StateProxy, StateApiType = S>,
    logger: &mut impl HasLogger,
) -> ContractResult<()> {
    // Only implementation can log event.
    only_implementation(host.state().implementation_address, ctx.sender())?;

    let mut parameter_buffer = vec![0; ctx.parameter_cursor().size() as usize];
    ctx.parameter_cursor().read_exact(&mut parameter_buffer)?;

    // Log event.
    logger.log(&RawReturnValue(parameter_buffer))?;

    Ok(())
}

// Contract functions

/// Initializes the state of the versus proxy contract with the state and the
/// implementation addresses. Both addresses have to be set together by calling
/// this function.
#[init(contract = "Versus-Proxy", parameter = "InitProxyParams")]
fn contract_proxy_init<S: HasStateApi>(
    ctx: &impl HasInitContext,
    _state_builder: &mut StateBuilder<S>,
) -> InitResult<StateProxy> {
    // Set state and implementation addresses.
    let params: InitProxyParams = ctx.parameter_cursor().get()?;

    // Get the instantiater of this contract instance.
    let invoker = Address::Account(ctx.init_origin());
    // Construct the initial proxy contract state.
    let state = StateProxy {
        admin:                  invoker,
        state_address:          params.state_address,
        implementation_address: params.implementation_address,
    };

    Ok(state)
}

/// Initializes the `implementation` and `state` contracts by using the
/// addresses that the `proxy` contract was set up. This function will call the
/// `initialize` functions on the `implementation` as well as the `state`
/// contracts. This function logs a mint event with amount 0 to signal that a
/// new CIS-2 token was deployed. This function logs an event including the
/// metadata for this token. This function logs a new implementation event.
/// This function logs a new admin event.
#[receive(
    contract = "Versus-Proxy",
    name = "initialize",
    error = "CustomContractError",
    enable_logger,
    mutable
)]
fn contract_proxy_initialize<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateProxy, StateApiType = S>,
    logger: &mut impl HasLogger,
) -> ContractResult<()> {
    let state_address = host.state().state_address;

    host.invoke_contract(
        &state_address,
        &InitializeStateParams {
            proxy_address:          ctx.self_address(),
            implementation_address: host.state().implementation_address,
        },
        EntrypointName::new_unchecked("initialize"),
        Amount::zero(),
    )?;

    let implementation_address = host.state().implementation_address;

    host.invoke_contract(
        &implementation_address,
        &InitializeImplementationParams {
            proxy_address: ctx.self_address(),
            state_address: host.state().state_address,
        },
        EntrypointName::new_unchecked("initialize"),
        Amount::zero(),
    )?;

    // Log a new implementation event.
    logger.log(&VersusEvent::NewImplementation(NewImplementationEvent {
        new_implementation: implementation_address,
    }))?;

    // Log a new admin event.
    logger.log(&VersusEvent::NewAdmin(NewAdminEvent {
        new_admin: host.state().admin,
    }))?;

    Ok(())
}

/// The fallback method, which redirects the invocations to the implementation.
#[receive(
    contract = "Versus-Proxy",
    error = "CustomContractError",
    fallback,
    mutable,
    payable
)]
fn receive_fallback<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateProxy, StateApiType = S>,
    amount: Amount,
) -> ReceiveResult<RawReturnValue> {
    let entrypoint = ctx.named_entrypoint();
    let implementation = host.state().implementation_address;

    let mut parameter_buffer = vec![0; ctx.parameter_cursor().size() as usize];
    ctx.parameter_cursor().read_exact(&mut parameter_buffer)?;

    // Forwarding the invoke unaltered to the implementation contract.
    let mut return_value = host
        .invoke_contract_raw(
            &implementation,
            Parameter(&parameter_buffer[..]),
            entrypoint.as_entrypoint_name(),
            amount,
        )
        .map_err(|r| {
            if let CallContractError::LogicReject {
                reason,
                mut return_value,
            } = r
            {
                let mut buffer = vec![0; return_value.size() as usize];
                return_value.read_exact(&mut buffer[..]).unwrap_abort(); // This should always be safe.
                let mut reject = Reject::new(reason).unwrap_abort();
                reject.return_value = Some(buffer);
                reject
            } else {
                r.into()
            }
        })?
        .1
        .unwrap_abort();

    let mut rv_buffer = vec![0; return_value.size() as usize];
    return_value.read_exact(&mut rv_buffer)?;
    Ok(RawReturnValue(rv_buffer))
}

// Simple helper functions to ensure that a call comes from the implementation
// or the proxy.

fn only_implementation(
    implementation_address: ContractAddress,
    sender: Address,
) -> ContractResult<()> {
    ensure!(
        sender.matches_contract(&implementation_address),
        CustomContractError::OnlyImplementation
    );

    Ok(())
}

/// Function to view state of the proxy contract.
#[receive(
    contract = "Versus-Proxy",
    name = "view",
    return_value = "StateProxy",
    error = "CustomContractError"
)]
fn contract_proxy_view<'a, 'b, S: HasStateApi>(
    _ctx: &'b impl HasReceiveContext,
    host: &'a impl HasHost<StateProxy, StateApiType = S>,
) -> ContractResult<&'a StateProxy> {
    Ok(host.state())
}

/// This functions allows the admin of the proxy to transfer the address to a
/// new admin.
#[receive(
    contract = "Versus-Proxy",
    name = "updateAdmin",
    parameter = "Address",
    error = "CustomContractError",
    enable_logger,
    mutable
)]
fn contract_proxy_update_admin<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateProxy, StateApiType = S>,
    logger: &mut impl HasLogger,
) -> ContractResult<()> {
    // Check that only the old admin is authorized to update the admin address.
    ensure_eq!(ctx.sender(), host.state().admin, CustomContractError::OnlyAdmin);
    // Parse the parameter.
    let new_admin = ctx.parameter_cursor().get()?;
    // Update admin.
    host.state_mut().admin = new_admin;

    // Log a new admin event.
    logger.log(&VersusEvent::NewAdmin(NewAdminEvent {
        new_admin,
    }))?;

    Ok(())
}

/// Function to update the protocol with a new implementation.
/// Only the admin on the proxy can call this function.
#[receive(
    contract = "Versus-Proxy",
    name = "updateImplementation",
    parameter = "SetImplementationAddressParams",
    error = "CustomContractError",
    enable_logger,
    mutable
)]
fn contract_proxy_update_implementation<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateProxy, StateApiType = S>,
    logger: &mut impl HasLogger,
) -> ContractResult<()> {
    // Check that only the proxy admin is authorized to update the implementation
    // address.
    ensure_eq!(ctx.sender(), host.state().admin, CustomContractError::OnlyAdmin);
    // Parse the parameter.
    let params: SetImplementationAddressParams = ctx.parameter_cursor().get()?;
    // Update implementation.
    host.state_mut().implementation_address = params.implementation_address;

    let state_address = host.state().state_address;

    // Update implementation address in the state contract.
    host.invoke_contract(
        &state_address,
        &SetImplementationAddressParams {
            implementation_address: params.implementation_address,
        },
        EntrypointName::new_unchecked("setImplementationAddress"),
        Amount::zero(),
    )?;

    // Log a new implementation event.
    logger.log(&VersusEvent::NewImplementation(NewImplementationEvent {
        new_implementation: params.implementation_address,
    }))?;

    Ok(())
}

// #[concordium_cfg_test]
// mod tests {
//     use super::*;
//     use test_infrastructure::*;

//     type ContractResult<A> = Result<A, Error>;

//     #[concordium_test]
//     /// Test that initializing the contract succeeds with some state.
//     fn test_init() {
//         let ctx = TestInitContext::empty();

//         let mut state_builder = TestStateBuilder::new();

//         let state_result = init(&ctx, &mut state_builder);
//         state_result.expect_report("Contract initialization results in error");
//     }

//     #[concordium_test]
//     /// Test that invoking the `receive` endpoint with the `false` parameter
//     /// succeeds in updating the contract.
//     fn test_throw_no_error() {
//         let ctx = TestInitContext::empty();

//         let mut state_builder = TestStateBuilder::new();

//         // Initializing state
//         let initial_state = init(&ctx, &mut state_builder).expect("Initialization should pass");

//         let mut ctx = TestReceiveContext::empty();

//         let throw_error = false;
//         let parameter_bytes = to_bytes(&throw_error);
//         ctx.set_parameter(&parameter_bytes);

//         let mut host = TestHost::new(initial_state, state_builder);

//         // Call the contract function.
//         let result: ContractResult<()> = receive(&ctx, &mut host);

//         // Check the result.
//         claim!(result.is_ok(), "Results in rejection");
//     }

//     #[concordium_test]
//     /// Test that invoking the `receive` endpoint with the `true` parameter
//     /// results in the `YourError` being thrown.
//     fn test_throw_error() {
//         let ctx = TestInitContext::empty();

//         let mut state_builder = TestStateBuilder::new();

//         // Initializing state
//         let initial_state = init(&ctx, &mut state_builder).expect("Initialization should pass");

//         let mut ctx = TestReceiveContext::empty();

//         let throw_error = true;
//         let parameter_bytes = to_bytes(&throw_error);
//         ctx.set_parameter(&parameter_bytes);

//         let mut host = TestHost::new(initial_state, state_builder);

//         // Call the contract function.
//         let error: ContractResult<()> = receive(&ctx, &mut host);

//         // Check the result.
//         claim_eq!(error, Err(Error::YourError), "Function should throw an error.");
//     }
// }
