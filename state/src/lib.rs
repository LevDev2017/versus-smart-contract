//! # A Concordium V1 smart contract
use concordium_std::*;
use core::fmt::Debug;

// Types

/// The state tracked for each address.
#[derive(Serialize, SchemaType)]
struct PlayerData {
    /// The player's state
    state:  PlayerState,
    /// The player's battle result
    result: BattleResult,
}

/// The `state` contract state.
#[derive(Serial, DeserialWithState, StateClone)]
#[concordium(state_parameter = "S")]
struct State<S> {
    /// Addresses of the protocol
    protocol_addresses: ProtocolAddressesState,
    /// The state of the one player.
    player_data:        StateMap<Address, PlayerData, S>,
    /// Contract is paused/unpaused.
    paused:             bool,
}

#[derive(Debug, Serialize, SchemaType)]
enum PlayerState {
    Active,
    Suspended
}

#[derive(Debug, Serialize, SchemaType)]
enum BattleResult {
    NoResult,
    Win,
    Loss
}

#[derive(Serialize, PartialEq, Clone)]
enum ProtocolAddressesState {
    UnInitialized,
    Initialized {
        /// Address of the w_ccd proxy contract.
        proxy_address:          ContractAddress,
        /// Address of the w_ccd implementation contract.
        implementation_address: ContractAddress,
    },
}

/// The parameter type for the state contract function `initialize`.
#[derive(Serialize, SchemaType)]
struct InitializeStateParams {
    /// Address of the w_ccd proxy contract.
    proxy_address:          ContractAddress,
    /// Address of the w_ccd implementation contract.
    implementation_address: ContractAddress,
}

/// The parameter type for the state contract function
/// `setImplementationAddress`.
#[derive(Serialize, SchemaType)]
struct SetImplementationAddressParams {
    /// Address of the w_ccd implementation contract.
    implementation_address: ContractAddress,
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

/// The parameter type for the state contract function `addPlayer`.
#[derive(Serialize, SchemaType)]
struct AddPlayerParams {
    /// Player to add.
    player: Address,
}

/// The parameter type for the state contract function `getPlayerData`.
#[derive(Serialize, SchemaType)]
struct GetPlayerDataParams {
    /// Player to get data
    player: Address,
}

/// The return type for the state contract function `view`.
#[derive(Serialize, SchemaType)]
struct ReturnBasicState {
    /// Address of the versus proxy contract.
    proxy_address:          ContractAddress,
    /// Address of the versus implementation contract.
    implementation_address: ContractAddress,
    /// Contract is paused/unpaused.
    paused:                 bool,
}

/// Your smart contract errors.
#[derive(Debug, PartialEq, Eq, Reject, Serial, SchemaType)]
enum CustomContractError {
    /// Failed parsing the parameter.
    #[from(ParseError)]
    ParseParamsError,
    /// Your error
    /// Failed to invoke a contract.
    InvokeContractError,
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
}

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

impl<S: HasStateApi> State<S> {
    /// Creates the new state of the `state` contract with no one having any
    /// data by default. The ProtocolAddressesState is uninitialized.
    /// The ProtocolAddressesState has to be set with the `initialize`
    /// function after the `proxy` contract is deployed.
    fn new(state_builder: &mut StateBuilder<S>) -> Self {
        // Setup state.
        State {
            protocol_addresses: ProtocolAddressesState::UnInitialized,
            player_data:        state_builder.new_map(),
            paused:             false,
        }
    }
}

// Contract functions

/// Init function that creates a new smart contract.
#[init(contract = "Versus-State")]
fn contract_state_init<S: HasStateApi>(
    _ctx: &impl HasInitContext,
    state_builder: &mut StateBuilder<S>,
) -> InitResult<State<S>> {
    // Construct the initial contract state.
    let state = State::new(state_builder);

    Ok(state)
}

/// Initializes the state versus contract with the proxy and the
/// implementation addresses. Both addresses have to be set together
/// by calling this function. This function can only be called once.
#[receive(
    contract = "Versus-State",
    name = "initialize",
    parameter = "InitializeStateParams",
    error = "CustomContractError",
    mutable
)]
fn contract_state_initialize<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    // Contract can only be initialized once.
    ensure_eq!(
        host.state().protocol_addresses,
        ProtocolAddressesState::UnInitialized,
        CustomContractError::AlreadyInitialized
    );

    // Set proxy and implementation addresses.
    let params: InitializeStateParams = ctx.parameter_cursor().get()?;

    host.state_mut().protocol_addresses = ProtocolAddressesState::Initialized {
        proxy_address:          params.proxy_address,
        implementation_address: params.implementation_address,
    };

    Ok(())
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

fn only_proxy(
    proxy_address: ContractAddress,
    sender: Address
) -> ContractResult<()> {
    ensure!(
        sender.matches_contract(&proxy_address),
        CustomContractError::OnlyProxy
    );

    Ok(())
}

/// Helper function to get protocol addresses from the state contract.
fn get_protocol_addresses_from_state<S>(
    host: &impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<(ContractAddress, ContractAddress)> {
    if let ProtocolAddressesState::Initialized {
        proxy_address,
        implementation_address,
    } = host.state().protocol_addresses
    {
        Ok((proxy_address, implementation_address))
    } else {
        bail!(CustomContractError::UnInitialized);
    }
}

// Getter and setter functions

/// Set implementation_address. Only the proxy can invoke this function.
/// The admin on the proxy will initiate the `updateImplementation` function on
/// the proxy which will invoke this function.
#[receive(
    contract = "Versus-State",
    name = "setImplementationAddress",
    parameter = "SetImplementationAddressParams",
    error = "CustomContractError",
    mutable
)]
fn contract_state_set_implementation_address<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let (proxy_address, _implementation_address) = get_protocol_addresses_from_state(host)?;

    // Only proxy can update the implementation address.
    only_proxy(proxy_address, ctx.sender())?;

    // Set implementation address.
    let params: SetImplementationAddressParams = ctx.parameter_cursor().get()?;

    host.state_mut().protocol_addresses = ProtocolAddressesState::Initialized {
        proxy_address,
        implementation_address: params.implementation_address,
    };

    Ok(())
}

/// Set paused.
#[receive(
    contract = "Versus-State",
    name = "setPaused",
    parameter = "SetPausedParams",
    error = "CustomContractError",
    mutable
)]
fn contract_state_set_paused<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let (_proxy_address, implementation_address) = get_protocol_addresses_from_state(host)?;

    // Only implementation can set state.
    only_implementation(implementation_address, ctx.sender())?;

    // Set paused.
    let params: SetPausedParams = ctx.parameter_cursor().get()?;
    host.state_mut().paused = params.paused;
    Ok(())
}

/// Update player state.
#[receive(
    contract = "Versus-State",
    name = "updatePlayerState",
    parameter = "UpdatePlayerStateParams",
    error = "CustomContractError",
    mutable
)]
fn contract_state_update_player_state<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let (_proxy_address, implementation_address) = get_protocol_addresses_from_state(host)?;

    // Only implementation can set state.
    only_implementation(implementation_address, ctx.sender())?;

    // update player state.
    let params: UpdatePlayerStateParams = ctx.parameter_cursor().get()?;
    let (state, _state_builder) = host.state_and_builder();

    let mut player_data = state.player_data.entry(params.player).or_insert_with(|| PlayerData {
        state:   PlayerState::Active,
        result:  BattleResult::NoResult,
    });
    player_data.state = params.state;

    // host.state_mut().player_data.entry(params.player).and_modify(|player_data| {
    //     player_data.state = params.state
    // })

    Ok(())
}

/// Update player battle result.
#[receive(
    contract = "Versus-State",
    name = "updateBattleResult",
    parameter = "UpdateBattleResultParams",
    error = "CustomContractError",
    mutable
)]
fn contract_state_update_battle_result<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let (_proxy_address, implementation_address) = get_protocol_addresses_from_state(host)?;

    // Only implementation can set result.
    only_implementation(implementation_address, ctx.sender())?;

    // update player state.
    let params: UpdateBattleResultParams = ctx.parameter_cursor().get()?;
    let (state, _state_builder) = host.state_and_builder();

    let mut player_data = state.player_data.entry(params.player).or_insert_with(|| PlayerData {
        state:   PlayerState::Active,
        result:  BattleResult::NoResult,
    });
    player_data.result = params.result;

    // host.state_mut().player_data.entry(params.player).and_modify(|player_data| {
    //     player_data.result = params.result
    // })

    Ok(())
}

/// Add new player with concordium id.
#[receive(
    contract = "Versus-State",
    name = "addPlayer",
    parameter = "AddPlayerParams",
    error = "CustomContractError",
    mutable
)]
fn contract_state_set_player_data<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let (_proxy_address, implementation_address) = get_protocol_addresses_from_state(host)?;

    // Only implementation can set result.
    only_implementation(implementation_address, ctx.sender())?;

    // add new player.
    let params: AddPlayerParams = ctx.parameter_cursor().get()?;
    let (state, _state_builder) = host.state_and_builder();

    let mut player_data = state.player_data.entry(params.player).or_insert_with(|| PlayerData {
        state:   PlayerState::Active,
        result:  BattleResult::NoResult,
    });

    Ok(())
}

/// Get paused.
#[receive(
    contract = "Versus-State",
    name = "getPaused",
    return_value = "bool",
    error = "CustomContractError"
)]
fn contract_state_get_paused<S: HasStateApi>(
    _ctx: &impl HasReceiveContext,
    host: &impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<bool> {
    Ok(host.state().paused)
}

/// Get player data.
#[receive(
    contract = "Versus-State",
    name = "getPlayerData",
    parameter = "GetPlayerDataParams",
    return_value = "(PlayerState, BattleResult)",
    error = "CustomContractError"
)]
fn contract_state_get_player_data<S: HasStateApi>(
    _ctx: &impl HasReceiveContext,
    host: &impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<(PlayerState, BattleResult)> {
    let params: AddPlayerParams = ctx.parameter_cursor().get()?;
    
    let player_data: PlayerData = host.state().player_data.entry(params.player);
    Ok((player_data.state, player_data.result))
}

/// Function to view state of the state contract.
#[receive(
    contract = "Versus-State",
    name = "view",
    return_value = "ReturnBasicState",
    error = "CustomContractError"
)]
fn contract_state_view<S: HasStateApi>(
    _ctx: &impl HasReceiveContext,
    host: &impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<ReturnBasicState> {
    let (proxy_address, implementation_address) = get_protocol_addresses_from_state(host)?;

    let state = ReturnBasicState {
        proxy_address,
        implementation_address,
        paused: host.state().paused,
    };
    Ok(state)
}

#[concordium_cfg_test]
mod tests {
    use super::*;
    use test_infrastructure::*;

    type ContractResult<A> = Result<A, Error>;

    #[concordium_test]
    /// Test that initializing the contract succeeds with some state.
    fn test_init() {
        let ctx = TestInitContext::empty();

        let mut state_builder = TestStateBuilder::new();

        let state_result = init(&ctx, &mut state_builder);
        state_result.expect_report("Contract initialization results in error");
    }

    #[concordium_test]
    /// Test that invoking the `receive` endpoint with the `false` parameter
    /// succeeds in updating the contract.
    fn test_throw_no_error() {
        let ctx = TestInitContext::empty();

        let mut state_builder = TestStateBuilder::new();

        // Initializing state
        let initial_state = init(&ctx, &mut state_builder).expect("Initialization should pass");

        let mut ctx = TestReceiveContext::empty();

        let throw_error = false;
        let parameter_bytes = to_bytes(&throw_error);
        ctx.set_parameter(&parameter_bytes);

        let mut host = TestHost::new(initial_state, state_builder);

        // Call the contract function.
        let result: ContractResult<()> = receive(&ctx, &mut host);

        // Check the result.
        claim!(result.is_ok(), "Results in rejection");
    }

    #[concordium_test]
    /// Test that invoking the `receive` endpoint with the `true` parameter
    /// results in the `YourError` being thrown.
    fn test_throw_error() {
        let ctx = TestInitContext::empty();

        let mut state_builder = TestStateBuilder::new();

        // Initializing state
        let initial_state = init(&ctx, &mut state_builder).expect("Initialization should pass");

        let mut ctx = TestReceiveContext::empty();

        let throw_error = true;
        let parameter_bytes = to_bytes(&throw_error);
        ctx.set_parameter(&parameter_bytes);

        let mut host = TestHost::new(initial_state, state_builder);

        // Call the contract function.
        let error: ContractResult<()> = receive(&ctx, &mut host);

        // Check the result.
        claim_eq!(error, Err(Error::YourError), "Function should throw an error.");
    }
}
