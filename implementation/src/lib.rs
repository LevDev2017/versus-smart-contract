//! # A Concordium V1 smart contract
use concordium_std::*;
use core::fmt::Debug;

/// Tag for the NewAdmin event. The CIS-2 library already uses the
/// event tags from `u8::MAX` to `u8::MAX - 4`.
pub const TOKEN_NEW_ADMIN_EVENT_TAG: u8 = u8::MAX - 5;

// Types

enum VersusEvent {
    /// A new admin event.
    NewAdmin(NewAdminEvent),
}

impl Serial for VersusEvent {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        match self {
            VersusEvent::NewAdmin(event) => {
                out.write_u8(TOKEN_NEW_ADMIN_EVENT_TAG)?;
                event.serial(out)
            }
        }
    }
}

/// The `implementation` contract state.
#[derive(Serial, Deserial, Clone, SchemaType)]
struct StateImplementation {
    /// The admin address can pause/unpause the contract
    admin:              Address,
    /// Addresses of the protocol
    protocol_addresses: ProtocolAddressesImplementation,
}

#[derive(Debug, Serialize, SchemaType, Clone, Copy)]
enum PlayerState {
    NotAdded,
    Active,
    Suspended
}

#[derive(Debug, Serialize, SchemaType, Clone, Copy)]
enum BattleResult {
    NoResult,
    Win,
    Loss
}

#[derive(SchemaType, Serialize, PartialEq, Clone)]
enum ProtocolAddressesImplementation {
    UnInitialized,
    Initialized {
        /// Address of the w_ccd proxy contract.
        proxy_address: ContractAddress,
        /// Address of the w_ccd state contract.
        state_address: ContractAddress,
    },
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

/// The parameter type for the implementation contract function `initialize`.
#[derive(Serialize, SchemaType)]
struct InitializeImplementationParams {
    /// Address of the w_ccd proxy contract.
    proxy_address: ContractAddress,
    /// Address of the w_ccd state contract.
    state_address: ContractAddress,
}

/// The parameter type for the state contract function `setPaused`.
#[derive(Serialize, SchemaType)]
struct SetPausedParams {
    /// Contract is paused/unpaused.
    paused: bool,
}

/// The parameter type for the state contract function `updatePlayerState`.
#[derive(Serialize, SchemaType)]
struct UpdatePlayerStateParams {
    /// Player to update state.
    player: Address,
    /// Active or Suspended
    state:  PlayerState,
}

/// The parameter type for the state contract function `updateBattleResult`.
#[derive(Serialize, SchemaType)]
struct UpdateBattleResultParams {
    /// Player to update state.
    player: Address,
    /// Win or Loss
    result: BattleResult,
}

/// Your smart contract errors.
#[derive(Debug, PartialEq, Eq, Reject, Serial, SchemaType)]
enum CustomContractError {
    /// Failed parsing the parameter.
    #[from(ParseError)]
    ParseParamsError,
    /// Failed logging: Log is full.
    LogFull,
    /// Failed logging: Log is malformed.
    LogMalformed,
    /// Failed to invoke a contract.
    InvokeContractError,
    /// Contract is paused.
    ContractPaused,
    /// Contract already initialized.
    AlreadyInitialized,
    /// Contract not initialized.
    UnInitialized,
    /// Only proxy contract.
    OnlyProxy,
    /// Raised when implementation/proxy can not invoke state contract.
    StateInvokeError,
    /// Only admin
    OnlyAdmin,
    /// Already added as player
    AlreadyAdded,
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

impl StateImplementation {
    /// Creates the new state of the `implementation` contract.
    /// The ProtocolAddressesState is uninitialized.
    /// The ProtocolAddressesState has to be set with the `initialize`
    /// function after the `proxy` contract is deployed.
    fn new(admin: Address) -> Self {
        // Setup state.
        StateImplementation {
            admin,
            protocol_addresses: ProtocolAddressesImplementation::UnInitialized,
        }
    }

    /// Check if an player is added in versus
    fn is_added<S>(
        &self,
        state_address: &ContractAddress,
        player: &Address,
        host: &impl HasHost<StateImplementation, StateApiType = S>,
    ) -> ContractResult<bool> {
        let is_added = host.invoke_contract_read_only(
            state_address,
            player,
            EntrypointName::new_unchecked("isAdded"),
            Amount::zero(),
        )?;
    
        let is_added = is_added.ok_or(CustomContractError::StateInvokeError)?.get()?;

        Ok(is_added)
    }
}

/// Initialize the implementation contract. This function logs a new admin
/// event.
#[init(contract = "Versus-Implementation", enable_logger)]
fn contract_init<S: HasStateApi>(
    ctx: &impl HasInitContext,
    _state_builder: &mut StateBuilder<S>,
    logger: &mut impl HasLogger,
) -> InitResult<StateImplementation> {
    // Get the instantiater of this contract instance.
    let invoker = Address::Account(ctx.init_origin());
    // Construct the initial contract state.
    let state = StateImplementation::new(invoker);

    // Log a new admin event.
    logger.log(&VersusEvent::NewAdmin(NewAdminEvent {
        new_admin: invoker,
    }))?;

    Ok(state)
}

/// Initializes the implementation versus contract with the proxy and
/// the state addresses. Both addresses have to be set together by
/// calling this function. This function can only be called once.
#[receive(
    contract = "Versus-Implementation",
    name = "initialize",
    parameter = "InitializeImplementationParams",
    error = "CustomContractError",
    mutable
)]
fn contract_initialize<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>,
) -> ContractResult<()> {
    // Contract can only be initialized once.
    ensure_eq!(
        host.state().protocol_addresses,
        ProtocolAddressesImplementation::UnInitialized,
        CustomContractError::AlreadyInitialized
    );

    // Set proxy and storage addresses.
    let params: InitializeImplementationParams = ctx.parameter_cursor().get()?;

    host.state_mut().protocol_addresses = ProtocolAddressesImplementation::Initialized {
        proxy_address: params.proxy_address,
        state_address: params.state_address,
    };

    Ok(())
}

// Simple helper functions to ensure that a call comes from the implementation
// or the proxy.

fn only_proxy(proxy_address: ContractAddress, sender: Address) -> ContractResult<()> {
    ensure!(
        sender.matches_contract(&proxy_address),
        CustomContractError::OnlyProxy
    );

    Ok(())
}

// Getter and setter functions

/// Function to view state of the implementation contract.
#[receive(
    contract = "Versus-Implementation",
    name = "view",
    return_value = "StateImplementation",
    error = "CustomContractError"
)]
fn contract_implementation_view<'a, 'b, S: HasStateApi>(
    _ctx: &'b impl HasReceiveContext,
    host: &'a impl HasHost<StateImplementation, StateApiType = S>,
) -> ContractResult<&'a StateImplementation> {
    Ok(host.state())
}

/// Helper function to get protocol addresses from the implementation contract.
fn get_protocol_addresses_from_implementation<S>(
    host: &impl HasHost<StateImplementation, StateApiType = S>,
) -> ContractResult<(ContractAddress, ContractAddress)> {
    if let ProtocolAddressesImplementation::Initialized {
        proxy_address,
        state_address,
    } = host.state().protocol_addresses
    {
        Ok((proxy_address, state_address))
    } else {
        bail!(CustomContractError::UnInitialized)
    }
}

/// Helper function to ensure contract is not paused.
fn when_not_paused<S>(
    state_address: &ContractAddress,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>,
) -> ContractResult<()> {
    let paused = host.invoke_contract_read_only(
        state_address,
        &Parameter(&[]),
        EntrypointName::new_unchecked("getPaused"),
        Amount::zero(),
    )?;

    // It is expected that this contract is initialized with the w_ccd_state
    // contract (a V1 contract). In that case, the paused variable can be
    // queried from the state contract without error.
    let paused: bool = paused
        .ok_or(CustomContractError::StateInvokeError)?
        .get()?;
    // Check that contract is not paused.
    ensure!(!paused, CustomContractError::ContractPaused);
    Ok(())
}

/// Update player state.
#[receive(
    contract = "Versus-Implementation",
    name = "updatePlayerState",
    parameter = "UpdatePlayerStateParams",
    error = "CustomContractError",
    mutable
)]
fn contract_implementation_update_player_state<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>
) -> ContractResult<()> {
    let (proxy_address, state_address) = get_protocol_addresses_from_implementation(host)?;

    // Can be only called through the fallback function on the proxy.
    only_proxy(proxy_address, ctx.sender())?;

    // Check that contract is not paused.
    when_not_paused(&state_address, host)?;

    // Parse the parameter.
    let input: UpdatePlayerStateParams = ctx.parameter_cursor().get()?;

    host.invoke_contract(
        &state_address,
        &UpdatePlayerStateParams {
            player: input.player,
            state: input.state,
        },
        EntrypointName::new_unchecked("updatePlayerState"),
        Amount::zero(),
    )?;

    // Log the update operator event.
    // host.invoke_contract(
    //     &proxy_address,
    //     &UpdateOperator(
    //         UpdateOperatorEvent {
    //             owner:    sender,
    //             operator: param.operator,
    //             update:   param.update,
    //         },
    //     ),
    //     EntrypointName::new_unchecked("logEvent"),
    //     Amount::zero(),
    // )?;

    Ok(())
}

/// Update battle result.
#[receive(
    contract = "Versus-Implementation",
    name = "updateBattleResult",
    parameter = "UpdateBattleResultParams",
    error = "CustomContractError",
    mutable
)]
fn contract_implementation_update_battle_result<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>
) -> ContractResult<()> {
    let (proxy_address, state_address) = get_protocol_addresses_from_implementation(host)?;

    // Can be only called through the fallback function on the proxy.
    only_proxy(proxy_address, ctx.sender())?;

    // Check that contract is not paused.
    when_not_paused(&state_address, host)?;

    // Parse the parameter.
    let input: UpdateBattleResultParams = ctx.parameter_cursor().get()?;

    host.invoke_contract(
        &state_address,
        &input,
        EntrypointName::new_unchecked("updateBattleResult"),
        Amount::zero(),
    )?;

    // Log the update operator event.
    // host.invoke_contract(
    //     &proxy_address,
    //     &UpdateOperator(
    //         UpdateOperatorEvent {
    //             owner:    sender,
    //             operator: param.operator,
    //             update:   param.update,
    //         },
    //     ),
    //     EntrypointName::new_unchecked("logEvent"),
    //     Amount::zero(),
    // )?;

    Ok(())
}

/// Add new player.
#[receive(
    contract = "Versus-Implementation",
    name = "addPlayer",
    parameter = "Address",
    error = "CustomContractError",
    mutable
)]
fn contract_implementation_add_player<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>
) -> ContractResult<()> {
    let (proxy_address, state_address) = get_protocol_addresses_from_implementation(host)?;

    // Can be only called through the fallback function on the proxy.
    only_proxy(proxy_address, ctx.sender())?;

    // Check that contract is not paused.
    when_not_paused(&state_address, host)?;

    // Parse the parameter.
    let input: Address = ctx.parameter_cursor().get()?;

    ensure!(
        host.state().is_added(&state_address, &input, host)?,
        CustomContractError::AlreadyAdded
    );

    host.invoke_contract(
        &state_address,
        &input,
        EntrypointName::new_unchecked("addPlayer"),
        Amount::zero(),
    )?;

    // Log the update operator event.
    // host.invoke_contract(
    //     &proxy_address,
    //     &UpdateOperator(
    //         UpdateOperatorEvent {
    //             owner:    sender,
    //             operator: param.operator,
    //             update:   param.update,
    //         },
    //     ),
    //     EntrypointName::new_unchecked("logEvent"),
    //     Amount::zero(),
    // )?;

    Ok(())
}

/// This functions allows the admin of the implementation to transfer the
/// address to a new admin.
#[receive(
    contract = "Versus-Implementation",
    name = "updateAdmin",
    parameter = "Address",
    error = "CustomContractError",
    enable_logger,
    mutable
)]
fn contract_implementation_update_admin<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>,
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

/// This function pauses the contract. Only the
/// admin of the implementation can call this function.
#[receive(
    contract = "Versus-Implementation",
    name = "pause",
    error = "CustomContractError",
    mutable
)]
fn contract_pause<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>,
) -> ContractResult<()> {
    // Check that only the current admin can pause.
    ensure_eq!(ctx.sender(), host.state().admin, CustomContractError::OnlyAdmin);

    let (_proxy_address, state_address) = get_protocol_addresses_from_implementation(host)?;

    host.invoke_contract(
        &state_address,
        &SetPausedParams {
            paused: true,
        },
        EntrypointName::new_unchecked("setPaused"),
        Amount::zero(),
    )?;

    Ok(())
}

/// Function to unpause the contract by the admin.
#[receive(
    contract = "Versus-Implementation",
    name = "unpause",
    error = "CustomContractError",
    mutable
)]
fn contract_un_pause<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>,
) -> ContractResult<()> {
    // Check that only the current admin can un_pause.
    ensure_eq!(ctx.sender(), host.state().admin, CustomContractError::OnlyAdmin);

    let (_proxy_address, state_address) = get_protocol_addresses_from_implementation(host)?;

    host.invoke_contract(
        &state_address,
        &SetPausedParams {
            paused: false,
        },
        EntrypointName::new_unchecked("setPaused"),
        Amount::zero(),
    )?;

    Ok(())
}

/// Get the player data
#[receive(
    contract = "Versus-Implementation",
    name = "getPlayerData",
    parameter = "Address",
    return_value = "(PlayerState, BattleResult)",
    error = "CustomContractError",
    mutable
)]
fn contract_implementation_get_player_data<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<StateImplementation, StateApiType = S>,
) -> ContractResult<(PlayerState, BattleResult)> {
    // Parse the parameter.
    let param: Address = ctx.parameter_cursor().get()?;
    let (_proxy_address, state_address) = get_protocol_addresses_from_implementation(host)?;

    let player_data = host.invoke_contract_read_only(
        &state_address,
        &param,
        EntrypointName::new_unchecked("getPlayerData"),
        Amount::zero(),
    )?;

    let (player_state, player_result) = player_data.ok_or(CustomContractError::StateInvokeError)?.get()?;

    Ok((player_state, player_result))
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
