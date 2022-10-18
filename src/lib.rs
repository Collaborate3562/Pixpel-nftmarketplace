#![cfg_attr(not(feature = "std"), no_std)]

use concordium_std::*;
use concordium_cis2::*;

#[derive(Serialize, Debug, PartialEq, Eq, Reject)]
pub enum MarketplaceError {
    ParseParams,
    CalledByAContract,
    TokenNotListed,
    Cis2ClientError(Cis2ClientError),
    CollectionNotCis2,
    InvalidAmountPaid,
    InvokeTransferError,
    NoBalance,
    NotOperator,
    NotMatchedSaleType,
    NotEnoughBalance,
    ExpiredAlready,
    CanNotBidYourSelf,
    CanceledAlready,
    Unauthorized,
    NotBidded,
}

#[derive(Serialize, Debug, PartialEq, Eq, Reject)]
pub enum Cis2ClientError {
    InvokeContractError,
    ParseParams,
    ParseResult,
}

pub const SUPPORTS_ENTRYPOINT_NAME: &str = "supports";
pub const OPERATOR_OF_ENTRYPOINT_NAME: &str = "operatorOf";
pub const BALANCE_OF_ENTRYPOINT_NAME: &str = "balanceOf";
pub const TRANSFER_ENTRYPOINT_NAME: &str = "transfer";

pub type ContractTokenAmount = TokenAmountU8;
type ContractBalanceOfQueryParams = BalanceOfQueryParams<ContractTokenId>;
type ContractBalanceOfQueryResponse = BalanceOfQueryResponse<ContractTokenAmount>;
type TransferParameter = TransferParams<ContractTokenId, ContractTokenAmount>;

type ContractResult<A> = Result<A, MarketplaceError>;

pub type ContractTokenId = TokenIdU32;

#[derive(Clone, Serialize, SchemaType)]
struct TokenInfo {
    pub id: ContractTokenId,
    pub address: ContractAddress,
}

impl TokenInfo {
    fn new(id: ContractTokenId, address: ContractAddress) -> Self {
        TokenInfo { id, address }
    }
}

#[derive(SchemaType, Clone, Serialize, Copy, PartialEq, Eq, Debug)]
enum TokenListState {
    UnListed,
    Listed,
}

#[derive(SchemaType, Clone, Serialize, Copy, PartialEq, Eq, Debug)]
enum TokenSaleTypeState {
    Fixed,
    Auction,
}

#[derive(Clone, Serialize, SchemaType)]
struct TokenState {
    sale_type: TokenSaleTypeState,
    curr_state: TokenListState,
    owner: AccountAddress,
    expiry: u64,
    highest_bidder: AccountAddress,
    price: Amount,
}

#[derive(Serial, DeserialWithState, StateClone)]
#[concordium(state_parameter = "S")]
struct State<S>
{
    tokens: StateMap<TokenInfo, TokenState, S>,
}

impl<S: HasStateApi> State<S> {
    fn new(state_builder: &mut StateBuilder<S>) -> Self {
        State {
            tokens: state_builder.new_map(),
        }
    }
}

#[init(contract = "Pixpel-NFTMarketplace")]
fn init<S: HasStateApi>(
    _ctx: &impl HasInitContext,
    state_builder: &mut StateBuilder<S>,
) -> InitResult<State<S>> {
    Ok(State::new(state_builder))
}

#[derive(Serial, Deserial, SchemaType)]
struct PlaceIntoMarketParams {
    nft_contract_address: ContractAddress,
    token_id: ContractTokenId,
    price: Amount,
    sale_type: u8,
    expiry: u64,
}

#[receive(
    contract = "Pixpel-NFTMarketplace",
    name = "place_into_market",
    parameter = "PlaceIntoMarketParams",
    mutable
)]
fn add<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let params: PlaceIntoMarketParams = ctx
        .parameter_cursor()
        .get()
        .map_err(|_e| MarketplaceError::ParseParams)?;

    ensure_supports_cis2(host, &params.nft_contract_address)?;
    ensure_is_operator(host, ctx, &params.nft_contract_address)?;
    ensure_balance(host, params.token_id, &params.nft_contract_address, ctx)?;

    let info = TokenInfo::new(params.token_id, params.nft_contract_address);
    let sale_type;
    if params.sale_type == 0 {
        sale_type = TokenSaleTypeState::Fixed;
    } else {
        sale_type = TokenSaleTypeState::Auction;
    }

    let curr_state = TokenListState::Listed;
    let owner = ctx.invoker();
    let highest_bidder = AccountAddress([0u8; 32]);
    let expiry = 0u64;
    let price = params.price;

    if host.state_mut().tokens.get(&info).is_some() {
        let mut token_state = host
            .state_mut()
            .tokens
            .entry(info)
            .occupied_or(MarketplaceError::TokenNotListed)?;
        token_state.owner = owner;
        token_state.highest_bidder = highest_bidder;
        token_state.sale_type = sale_type;
        token_state.curr_state = curr_state;
        token_state.expiry = params.expiry;
        token_state.price = params.price;
    } else {
        host.state_mut().tokens.insert(
            info,
            TokenState {
                sale_type,
                curr_state,
                owner,
                expiry,
                highest_bidder,
                price
            },
        );
    }
    ContractResult::Ok(())
}

#[derive(Serial, Deserial, SchemaType)]
struct TradeNftParams {
    nft_contract_address: ContractAddress,
    token_id: ContractTokenId,
    price: Amount,
    sale_type: u8
}

#[receive(
    contract = "Pixpel-NFTMarketplace",
    name = "trade_market",
    parameter = "TradeNftParams",
    mutable,
    payable
)]
fn trade_nft<S:HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
    amount: Amount
) -> ContractResult<()> {
    let params: TradeNftParams = ctx
        .parameter_cursor()
        .get()
        .map_err(|_e| MarketplaceError::ParseParams)?;

    let info = TokenInfo::new(params.token_id, params.nft_contract_address);
    ensure!(host.state_mut().tokens.get(&info).is_some(), MarketplaceError::TokenNotListed);

    let mut token_state = host
        .state_mut()
        .tokens
        .entry(info)
        .occupied_or(MarketplaceError::TokenNotListed)?.to_owned();

    let price = token_state.price;
    ensure!(
        amount.cmp(&price).is_gt(),
        MarketplaceError::NotEnoughBalance
    );
    
    if params.sale_type == 0 {
        ensure!(token_state.sale_type == TokenSaleTypeState::Fixed, MarketplaceError::NotMatchedSaleType);

        Cis2Client::transfer(
            host,
            params.token_id,
            params.nft_contract_address,
            concordium_cis2::TokenAmountU8(1),
            token_state.owner,
            concordium_cis2::Receiver::Account(ctx.invoker()),
        )
        .map_err(MarketplaceError::Cis2ClientError)?;

        host.invoke_transfer(&token_state.owner, amount)
            .map_err(|_| MarketplaceError::InvokeTransferError)?;
            
        token_state.owner = ctx.invoker();
        token_state.sale_type = TokenSaleTypeState::Fixed;
        token_state.curr_state = TokenListState::UnListed;
        token_state.expiry = 0u64;
        token_state.highest_bidder = AccountAddress([0u8;32]);
        token_state.price = Amount { micro_ccd: 0u64 };
    } else if params.sale_type == 1 {
        ensure!(token_state.sale_type == TokenSaleTypeState::Auction, MarketplaceError::NotMatchedSaleType);

        let slot_time = ctx.metadata().slot_time();

        ensure!(concordium_std::Timestamp::timestamp_millis(&slot_time) <= token_state.expiry, MarketplaceError::ExpiredAlready);
        ensure!(ctx.invoker() != token_state.owner, MarketplaceError::CanNotBidYourSelf);
        if token_state.highest_bidder != AccountAddress([0u8; 32]) {
            host.invoke_transfer(&token_state.highest_bidder, token_state.price )
            .map_err(|_| MarketplaceError::InvokeTransferError)?;
        }

        token_state.highest_bidder = ctx.invoker();
        token_state.price = amount;
    }

    ContractResult::Ok(())
}

#[derive(Serial, Deserial, SchemaType)]
struct CancelTradeParams {
    nft_contract_address: ContractAddress,
    token_id: ContractTokenId,
    sale_type: u8,
}

#[receive(
    contract = "Pixpel-NFTMarketplace",
    name = "cancel_trade",
    parameter = "CancelTradeParams",
    mutable
)]
fn cancel_trade<S:HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>
) -> ContractResult<()> {
    let params: CancelTradeParams = ctx
        .parameter_cursor()
        .get()
        .map_err(|_e| MarketplaceError::ParseParams)?;

    let info = TokenInfo::new(params.token_id, params.nft_contract_address);
    ensure!(host.state_mut().tokens.get(&info).is_some(), MarketplaceError::TokenNotListed);
    
    let mut token_state = host
        .state_mut()
        .tokens
        .entry(info)
        .occupied_or(MarketplaceError::TokenNotListed)?.to_owned();
        
    ensure!(token_state.curr_state == TokenListState::Listed, MarketplaceError::CanceledAlready);
    let sender = ctx.sender();
    ensure!(
        sender.matches_account(&token_state.owner),
        MarketplaceError::Unauthorized
    );

    if params.sale_type == 0 {
        ensure!(token_state.sale_type == TokenSaleTypeState::Fixed, MarketplaceError::NotMatchedSaleType);   
    } else if params.sale_type == 1 {
        ensure!(token_state.sale_type == TokenSaleTypeState::Auction, MarketplaceError::NotMatchedSaleType);
    }

    token_state.sale_type = TokenSaleTypeState::Fixed;
    token_state.curr_state = TokenListState::UnListed;
    token_state.expiry = 0u64;
    token_state.highest_bidder = AccountAddress([0u8; 32]);
    token_state.price = Amount { micro_ccd: 0u64 };

    ContractResult::Ok(())
}

#[derive(Serial, Deserial, SchemaType)]
struct FinaliseTradeParams {
    nft_contract_address: ContractAddress,
    token_id: ContractTokenId,
    sale_type: u8,
}

#[receive(
    contract = "Pixpel-NFTMarketplace",
    name = "finalise_trade",
    parameter = "FinaliseTradeParams",
    mutable
)]
fn finalise_trade<S:HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>
) -> ContractResult<()> {
    let params: FinaliseTradeParams = ctx
        .parameter_cursor()
        .get()
        .map_err(|_e| MarketplaceError::ParseParams)?;
    
    ensure!(params.sale_type.cmp(&1u8).is_ge(), MarketplaceError::NotMatchedSaleType);
    
    let info = TokenInfo::new(params.token_id, params.nft_contract_address);
    ensure!(host.state_mut().tokens.get(&info).is_some(), MarketplaceError::TokenNotListed);
    
    let mut token_state = host
        .state_mut()
        .tokens
        .entry(info)
        .occupied_or(MarketplaceError::TokenNotListed)?.to_owned();
    
    let sender = ctx.sender();
    ensure!(
        sender.matches_account(&token_state.owner),
        MarketplaceError::Unauthorized  
    );

    if token_state.highest_bidder != AccountAddress([0u8; 32]) {
        host.invoke_transfer(&token_state.owner, token_state.price )
            .map_err(|_| MarketplaceError::InvokeTransferError)?;

        Cis2Client::transfer(
            host,
            params.token_id,
            params.nft_contract_address,
            concordium_cis2::TokenAmountU8(1),
            token_state.owner,
            concordium_cis2::Receiver::Account(ctx.invoker()),
        )
        .map_err(MarketplaceError::Cis2ClientError)?;

        token_state.owner = ctx.invoker();
        token_state.sale_type = TokenSaleTypeState::Fixed;
        token_state.curr_state = TokenListState::UnListed;
        token_state.expiry = 0u64;
        token_state.highest_bidder = AccountAddress([0u8; 32]);
        token_state.price = Amount { micro_ccd: 0u64 };
    } else {
        bail!(MarketplaceError::NotBidded)
    }

    ContractResult::Ok(())
}

pub struct Cis2Client;

impl Cis2Client {
    pub(crate) fn supports_cis2<S: HasStateApi>(
        host: &mut impl HasHost<State<S>, StateApiType = S>,
        nft_contract_address: &ContractAddress,
    ) -> Result<bool, Cis2ClientError> {
        let params = SupportsQueryParams {
            queries: vec![StandardIdentifierOwned::new_unchecked("CIS-2".to_string())],
        };
        let parsed_res: SupportsQueryResponse = Cis2Client::invoke_contract_read_only(
            host,
            nft_contract_address,
            SUPPORTS_ENTRYPOINT_NAME,
            &params,
        )?;
        let supports_cis2: bool = {
            let f = parsed_res
                .results
                .first()
                .ok_or(Cis2ClientError::InvokeContractError)?;
            match f {
                SupportResult::NoSupport => false,
                SupportResult::Support => true,
                SupportResult::SupportBy(_) => false,
            }
        };

        Ok(supports_cis2)
    }

    pub(crate) fn is_operator_of<S: HasStateApi>(
        host: &mut impl HasHost<State<S>, StateApiType = S>,
        owner: Address,
        current_contract_address: ContractAddress,
        nft_contract_address: &ContractAddress,
    ) -> Result<bool, Cis2ClientError> {
        let params = &OperatorOfQueryParams {
            queries: vec![OperatorOfQuery {
                owner,
                address: Address::Contract(current_contract_address),
            }],
        };

        let parsed_res: OperatorOfQueryResponse = Cis2Client::invoke_contract_read_only(
            host,
            nft_contract_address,
            OPERATOR_OF_ENTRYPOINT_NAME,
            params,
        )?;

        let is_operator = parsed_res
            .0
            .first()
            .ok_or(Cis2ClientError::InvokeContractError)?
            .to_owned();

        Ok(is_operator)
    }

    pub(crate) fn has_balance<S: HasStateApi>(
        host: &mut impl HasHost<State<S>, StateApiType = S>,
        token_id: ContractTokenId,
        nft_contract_address: &ContractAddress,
        owner: Address,
    ) -> Result<bool, Cis2ClientError> {
        let params = ContractBalanceOfQueryParams {
            queries: vec![BalanceOfQuery {
                token_id,
                address: owner,
            }],
        };

        let parsed_res: ContractBalanceOfQueryResponse = Cis2Client::invoke_contract_read_only(
            host,
            nft_contract_address,
            BALANCE_OF_ENTRYPOINT_NAME,
            &params,
        )?;

        let is_operator = parsed_res
            .0
            .first()
            .ok_or(Cis2ClientError::InvokeContractError)?
            .to_owned();

        Result::Ok(is_operator.cmp(&TokenAmountU8(1)).is_ge())
    }

    pub(crate) fn transfer<S: HasStateApi>(
        host: &mut impl HasHost<State<S>, StateApiType = S>,
        token_id: TokenIdU32,
        nft_contract_address: ContractAddress,
        amount: ContractTokenAmount,
        from: AccountAddress,
        to: Receiver,
    ) -> Result<bool, Cis2ClientError> {
        let params: TransferParameter = TransferParams(vec![Transfer {
            token_id,
            amount,
            from: concordium_std::Address::Account(from),
            data: AdditionalData::empty(),
            to,
        }]);

        Cis2Client::invoke_contract_read_only(
            host,
            &nft_contract_address,
            TRANSFER_ENTRYPOINT_NAME,
            &params,
        )?;

        Result::Ok(true)
    }

    fn invoke_contract_read_only<S: HasStateApi, R: Deserial, P: Serial>(
        host: &mut impl HasHost<State<S>, StateApiType = S>,
        contract_address: &ContractAddress,
        entrypoint_name: &str,
        params: &P,
    ) -> Result<R, Cis2ClientError> {
        let invoke_contract_result = host
            .invoke_contract_read_only(
                contract_address,
                params,
                EntrypointName::new(entrypoint_name).unwrap_abort(),
                Amount::from_ccd(0),
            )
            .map_err(|_e| Cis2ClientError::InvokeContractError)?;
        let mut invoke_contract_res = match invoke_contract_result {
            Some(s) => s,
            None => return Result::Err(Cis2ClientError::InvokeContractError),
        };
        let parsed_res =
            R::deserial(&mut invoke_contract_res).map_err(|_e| Cis2ClientError::ParseResult)?;

        Ok(parsed_res)
    }
}

fn ensure_supports_cis2<S: HasStateApi>(
    host: &mut impl HasHost<State<S>, StateApiType = S>,
    nft_contract_address: &ContractAddress,
) -> Result<(), MarketplaceError> {
    let supports_cis2 = Cis2Client::supports_cis2(host, nft_contract_address)
        .map_err(MarketplaceError::Cis2ClientError)?;
    ensure!(supports_cis2, MarketplaceError::CollectionNotCis2);
    Ok(())
}

fn ensure_is_operator<S: HasStateApi>(
    host: &mut impl HasHost<State<S>, StateApiType = S>,
    ctx: &impl HasReceiveContext<()>,
    nft_contract_address: &ContractAddress,
) -> Result<(), MarketplaceError> {
    let is_operator = Cis2Client::is_operator_of(
        host,
        ctx.sender(),
        ctx.self_address(),
        nft_contract_address,
    )
    .map_err(MarketplaceError::Cis2ClientError)?;
    ensure!(is_operator, MarketplaceError::NotOperator);
    Ok(())
}

fn ensure_balance<S: HasStateApi>(
    host: &mut impl HasHost<State<S>, StateApiType = S>,
    token_id: ContractTokenId,
    nft_contract_address: &ContractAddress,
    ctx: &impl HasReceiveContext<()>,
) -> Result<(), MarketplaceError> {
    let has_balance = Cis2Client::has_balance(host, token_id, nft_contract_address, ctx.sender())
        .map_err(MarketplaceError::Cis2ClientError)?;
    ensure!(has_balance, MarketplaceError::NoBalance);
    Ok(())
}
