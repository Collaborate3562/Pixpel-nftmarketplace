#![cfg_attr(not(feature = "std"), no_std)]

use concordium_cis2::*;
use concordium_std::*;
use core::fmt::Debug;

#[derive(Serialize, Debug, PartialEq, Eq, Reject)]
pub enum MarketplaceError {
    #[from(ParseError)]
    ParseParams,
    TokenNotFound,
    Unauthorized,
    InvokeContractError,
}

pub type ContractError = Cis2Error<MarketplaceError>;
pub type ContractResult<A> = Result<A, ContractError>;

impl<T> From<CallContractError<T>> for MarketplaceError {
    fn from(_e: CallContractError<T>) -> Self {
        MarketplaceError::InvokeContractError
    }
}

impl From<MarketplaceError> for ContractError {
    fn from(c: MarketplaceError) -> Self {
        Cis2Error::Custom(c)
    }
}

#[derive(Debug, SchemaType, Eq, PartialEq)]
pub struct ParamWithSender<T> {
    pub params: T,
    pub sender: Address,
}

impl Serial for ParamWithSender<Vec<u8>> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        out.write_all(&self.params)?;
        self.sender.serial(out)
    }
}

impl<T: Deserial> Deserial for ParamWithSender<T> {
    fn deserial<R: Read>(source: &mut R) -> ParseResult<Self> {
        let params = T::deserial(source)?;
        let sender = Address::deserial(source)?;
        Ok(ParamWithSender {
            params,
            sender,
        })
    }
}

#[derive(PartialEq, Eq, Debug)]
struct RawReturnValue(Vec<u8>);

impl Serial for RawReturnValue {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> { out.write_all(&self.0) }
}

type TokenId = TokenIdU32;
type TokenPrice = TokenAmountU32;

// #[derive(Debug, Serialize, SchemaType, Eq, PartialEq, PartialOrd)]
// pub enum SaleType {
//     Auction,
//     FixedSale,
// }

#[derive(Debug, Serialize, SchemaType, Eq, PartialEq, PartialOrd)]
pub enum PurchaseState {
    Sold,
    NotSoldYet
}

#[derive(Debug, Serialize, SchemaType, Eq, PartialEq)]
pub struct MarketItem {
    creator: AccountAddress,
    price: TokenPrice,
    state: PurchaseState
}

#[derive(Debug, Serialize, SchemaType, Eq, PartialEq)]
struct AuctionItem {
    expiry: Timestamp,
    state: PurchaseState,
    highest_bid: TokenPrice,
    creator: AccountAddress,
    highest_bidder: AccountAddress,
    #[concordium(size_length = 2)]
    bids: collections::BTreeMap<AccountAddress, Amount>,
}

#[derive(Serial, DeserialWithState, Deletable)]
#[concordium(state_parameter = "S")]
struct State<S> {
    tokens_for_sale: StateMap<TokenId, MarketItem, S>,
    tokens_for_auction: StateMap<TokenId, AuctionItem, S>
}

impl<S: HasStateApi> State<S> {
    fn empty(state_builder: &mut StateBuilder<S>) -> State<S> {
        State {
            tokens_for_sale: state_builder.new_map(),
            tokens_for_auction: state_builder.new_map(),
        }
    }
}

#[init(contract = "pixpel-nftmarketplace")]
fn init_marketplace<S: HasStateApi>(
    _ctx: &impl HasInitContext,
    state_builder: &mut StateBuilder<S>,
) -> ContractResult<State<S>> {
    Ok(State::empty(state_builder))
}

#[derive(SchemaType, Serial, Deserial)]
struct PlaceForSaleParameter {
    token_id: TokenId,
    price: TokenPrice,
    pixpel_nft: ContractAddress,
}

impl Serial for ParamWithSender<PlaceForSaleParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_id.serial(out)?;
        self.params.price.serial(out)?;
        self.params.pixpel_nft.serial(out)?;
        self.sender.serial(out)
    }
}

// type TransferParameter = TransferParams<TokenId, ContractTokenAmount>;

#[receive(
    contract = "pixpel-nftmarketplace",
    name = "open_trade",
    parameter = "PlaceForSaleParameter",
    mutable
)]
fn marketplace_place_for_sale<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let input: ParamWithSender<PlaceForSaleParameter> = ctx.parameter_cursor().get()?;
    let param = input.params;

    let sender = input.sender;
    let owner = ctx.owner();

    ensure!(
        sender.matches_account(&owner),
        MarketplaceError::Unauthorized.into()
    );

    let transfer = Transfer::<TokenId, TokenPrice> {
        token_id: param.token_id,
        amount: 1.into(),
        from: Address::Account(owner),
        to: Receiver::ContractAddress(ctx.self_address()),
        data: AdditionalData::empty(),
    };

    let parameter = TransferParams::from(vec![transfer]);

    host.invoke_contract(
        &(param.pixpel_nft),
        &parameter,
        EntrypointName::new_unchecked("transfer"),
        Amount::zero(),
    );

    let state = host.state_mut();

    state.tokens_for_sale.insert(param.token_id, {
        MarketItem {
            creator: ctx.invoker(),
            price: param.price,
            state: PurchaseState::NotSoldYet
        }
    });
    
    Ok(())
}

#[derive(SchemaType, Serial, Deserial)]
struct PlaceForAuctionParameter {
    token_id: TokenId,
    price: TokenPrice,
    expiry: Timestamp,
}

impl Serial for ParamWithSender<PlaceForAuctionParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_id.serial(out)?;
        self.params.price.serial(out)?;
        self.params.expiry.serial(out)?;
        self.sender.serial(out)
    }
}


#[receive(
    contract = "pixpel-nftmarketplace",
    name = "create_auction",
    parameter = "PlaceForAuctionParameter",
    mutable
)]
fn marketplace_place_for_auction<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let input: ParamWithSender<PlaceForAuctionParameter> = ctx.parameter_cursor().get()?;
    let param = input.params;

    let sender = input.sender;
    let owner = ctx.owner();

    ensure!(
        sender.matches_account(&owner),
        MarketplaceError::Unauthorized.into()
    );

    let state = host.state_mut();

    state.tokens_for_auction.insert(param.token_id, {
        AuctionItem {
            expiry: param.expiry,
            state: PurchaseState::NotSoldYet,
            highest_bid: param.price,
            creator: ctx.invoker(),
            highest_bidder: ctx.invoker(),
            bids: collections::BTreeMap::new()
        }
    });
    
    Ok(())
}

#[derive(SchemaType, Serial, Deserial)]
struct CancelForTradeParameter {
    token_id: TokenId,
}

impl Serial for ParamWithSender<CancelForTradeParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_id.serial(out)?;
        self.sender.serial(out)
    }
}

#[receive(
    contract = "pixpel-nftmarketplace",
    name = "cancel_trade",
    parameter = "CancelForTradeParameter",
    mutable
)]
fn marketplace_cancel_for_trade<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let input: ParamWithSender<CancelForTradeParameter> = ctx.parameter_cursor().get()?;
    let param = input.params;

    let sender = input.sender;
    let owner = ctx.owner();

    ensure!(
        sender.matches_account(&owner),
        MarketplaceError::Unauthorized.into()
    );

    let state = host.state_mut();

    let market_item = state.tokens_for_sale.get(&param.token_id);

    // ensure!(
    //     ctx.invoker() != market_item.creator,
    //     MarketplaceError::Unauthorized.into()
    // );

    ensure!(
        state.tokens_for_sale.get(&param.token_id).is_some(),
        MarketplaceError::TokenNotFound.into()
    );

    state.tokens_for_sale.remove(&param.token_id);
    Ok(())
}

#[derive(SchemaType, Serial, Deserial)]
struct BidForNftParameter {
    token_id: TokenId,
    price: TokenPrice
}

impl Serial for ParamWithSender<BidForNftParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_id.serial(out)?;
        self.params.price.serial(out)?;
        self.sender.serial(out)
    }
}

// #[receive(
//     contract = "pixpel-nftmarketplace",
//     name = "bid",
//     parameter = "BidForNftParameter",
//     mutable
// )]
// fn marketplace_bid_for_nft<S: HasStateApi>(
//     ctx: &impl HasReceiveContext,
//     host: &mut impl HasHost<State<S>, StateApiType = S>,
// ) -> ContractResult<()> {
//     let input: ParamWithSender<BidForNftParameter> = ctx.parameter_cursor().get()?;
//     let param = input.params;

//     let sender = input.sender;
//     let owner = ctx.owner();

//     ensure!(
//         sender.matches_account(&owner),
//         MarketplaceError::Unauthorized.into()
//     );

//     let state = host.state_mut();

//     let market_item = state.tokens_for_auction.get(&param.token_id);

//     state.tokens_for_auction.

//     ensure!(
//         state.tokens_for_auction.get(&param.token_id).is_some(),
//         MarketplaceError::TokenNotFound.into()
//     );

//     Ok(())
// }