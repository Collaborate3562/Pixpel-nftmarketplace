use concordium_cis2::*;
use concordium_std::*;

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

#[derive(Debug)]
pub struct ParamWithSender<T> {
    pub params: T,
    pub sender: Address,
}


//impl<T: Serial> Serial for ParamWithSender<T> {
//    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
//        self.params.serial(out)?;
//        self.sender.serial(out)
//    }
//}

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

/*
 *    Marketplace contract implementation
 */

type TokenId = TokenIdU32;
type TokenPrice = TokenAmountU32;

#[derive(Serial, DeserialWithState, Deletable)]
#[concordium(state_parameter = "S")]
struct State<S> {
    tokens_for_sale: StateMap<TokenId, TokenPrice, S>,
}

impl<S: HasStateApi> State<S> {
    fn empty(state_builder: &mut StateBuilder<S>) -> State<S> {
        State {
            tokens_for_sale: state_builder.new_map(),
        }
    }
}

#[init(contract = "pixpel-nftmarketplace")]
fn marketplace_init<S: HasStateApi>(
    _ctx: &impl HasInitContext,
    state_builder: &mut StateBuilder<S>,
) -> ContractResult<State<S>> {
    Ok(State::empty(state_builder))
}


#[derive(SchemaType, Serial, Deserial)]
struct PlaceForSaleParameter {
    token_id: TokenId,
    price: TokenPrice,
}

impl Serial for ParamWithSender<PlaceForSaleParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_id.serial(out)?;
        self.params.price.serial(out)?;
        self.sender.serial(out)
    }
}

#[receive(
    contract = "pixpel-nftmarketplace",
    name = "place_for_sale",
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

    let state = host.state_mut();
    state.tokens_for_sale.insert(param.token_id, param.price);
    
    Ok(())
}

#[derive(Serial, SchemaType, Clone, PartialEq)]
struct ViewState {
    tokens: Vec<ViewStateToken>,
}

#[derive(Serial, SchemaType, Clone, PartialEq)]
struct ViewStateToken {
    id: TokenId,
    price: TokenPrice,
}

#[receive(
    contract = "pixpel-nftmarketplace",
    name = "view_list_for_sale",
    return_value = "ViewState"
)]
fn marketplace_view_list_for_sale<S: HasStateApi>(
    _ctx: &impl HasReceiveContext,
    host: &impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<ViewState> {
    let mut view_state = ViewState { tokens: Vec::new() };

    for (id, amount) in host.state().tokens_for_sale.iter() {
        view_state.tokens.push(ViewStateToken {
            id: *id,
            price: *amount,
        });
    }

    Ok(view_state)
}

#[derive(SchemaType, Serialize)]
struct GetListedParameter {
    token_ids: Vec<TokenId>,
}

impl Serial for ParamWithSender<GetListedParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_ids.serial(out)?;
        self.sender.serial(out)
    }
}

#[receive(
    contract = "pixpel-nftmarketplace",
    name = "get_listed_for_sale",
    parameter = "GetListedParameter",
    return_value = "ViewState"
)]
fn marketplace_get_listed_for_sale<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<ViewState> {
    let param: GetListedParameter = ctx.parameter_cursor().get()?;

    let mut view_state = ViewState { tokens: Vec::new() };
    let state = host.state();

    for id in param.token_ids {
        if let Some(amount) = state.tokens_for_sale.get(&id) {
            view_state.tokens.push(ViewStateToken {
                id, 
                price: *amount,
            });
        }
    }

    Ok(view_state)
}



#[derive(SchemaType, Serialize)]
struct WithdrawParameter {
    token_id: TokenId,
}

impl Serial for ParamWithSender<WithdrawParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_id.serial(out)?;
        self.sender.serial(out)
    }
}

#[receive(
    contract = "pixpel-nftmarketplace",
    name = "withdraw",
    parameter = "WithdrawParameter",
    mutable
)]
fn marketplace_withdraw<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let input: ParamWithSender<WithdrawParameter> = ctx.parameter_cursor().get()?;
    let param = input.params;

    let sender = input.sender;
    let owner = ctx.owner();

    ensure!(
        sender.matches_account(&owner),
        MarketplaceError::Unauthorized.into()
    );

    let state = host.state_mut();

    ensure!(
        state.tokens_for_sale.get(&param.token_id).is_some(),
        MarketplaceError::TokenNotFound.into()
    );

    state.tokens_for_sale.remove(&param.token_id);
    Ok(())
}

#[derive(SchemaType, Serialize)]
struct PurchaseParameter {
    token_id: TokenId,
    from: AccountAddress,
    to: AccountAddress,
    contract: ContractAddress,
}

impl Serial for ParamWithSender<PurchaseParameter> {
    fn serial<W: Write>(&self, out: &mut W) -> Result<(), W::Err> {
        self.params.token_id.serial(out)?;
        self.params.from.serial(out)?;
        self.params.to.serial(out)?;
        self.params.contract.serial(out)?;
        self.sender.serial(out)
    }
}

#[receive(
    contract = "pixpel-nftmarketplace",
    name = "purchase",
    parameter = "PurchaseParameter",
    mutable
)]
fn marketplace_purchase<S: HasStateApi>(
    ctx: &impl HasReceiveContext,
    host: &mut impl HasHost<State<S>, StateApiType = S>,
) -> ContractResult<()> {
    let input: ParamWithSender<PurchaseParameter> = ctx.parameter_cursor().get()?;
    let purchase = input.params;

    let sender = input.sender;
    let owner = ctx.owner();

    ensure!(
        sender.matches_account(&owner),
        MarketplaceError::Unauthorized.into()
    );

    let state = host.state_mut();
    let token = state.tokens_for_sale.remove_and_get(&purchase.token_id);
    ensure!(token.is_some(), MarketplaceError::TokenNotFound.into());

    let transfer = Transfer::<TokenId, TokenPrice> {
        token_id: purchase.token_id,
        amount: 1.into(),
        from: Address::Account(purchase.from),
        to: Receiver::Account(purchase.to),
        data: AdditionalData::empty(),
    };

    let parameter = TransferParams::from(vec![transfer]);

    host.invoke_contract(
        &(purchase.contract),
        &parameter,
        EntrypointName::new_unchecked("transfer"),
        Amount::zero(),
    )?;

    Ok(())
}

#[concordium_cfg_test]
mod tests {
    use super::*;
    use concordium_std::test_infrastructure::*;

    const OWNER: AccountAddress = AccountAddress([0u8; 32]);
    const OWNER_ADDR: Address = Address::Account(OWNER);

    const RECEIVER: AccountAddress = AccountAddress([1u8; 32]);

    const NFT_CONTRACT: ContractAddress = ContractAddress{index: 42, subindex: 0};

    const TOKEN1_ID: TokenId = TokenIdU32(1);
    const TOKEN1_PRICE: TokenPrice = TokenAmountU32(1000);

    const TOKEN2_ID: TokenId = TokenIdU32(2);

    #[concordium_test]
    fn test_init() {
        // Setup the context
        let ctx = TestInitContext::empty();
        let mut builder = TestStateBuilder::new();

        // Call the contract function.
        let result = marketplace_init(&ctx, &mut builder);

        // Check the result
        let state = result.expect_report("Contract initialization failed");

        // Check the state
        claim_eq!(state.tokens_for_sale.iter().count(), 0, "No token should be listed for sale after initialization.");
    }

    #[concordium_test]
    fn test_place_for_sale() {
        let mut ctx = TestReceiveContext::empty();
        ctx.set_owner(OWNER);
        ctx.set_sender(OWNER_ADDR);

        let mut state_builder = TestStateBuilder::new();
        let mut host = TestHost::new(State::empty(&mut state_builder), state_builder);

        let param = PlaceForSaleParameter {
            token_id: 1.into(),
            price: 1000.into(),
        };

        let param_with_sender: ParamWithSender<PlaceForSaleParameter> = ParamWithSender {
            sender: OWNER_ADDR,
            params: param,
        };

        let param_bytes = to_bytes(&param_with_sender);
        ctx.set_parameter(&param_bytes);

        let result = marketplace_place_for_sale(&ctx, &mut host);
        claim!(result.is_ok(), "Place for sale results in rejection.");

        claim_eq!(host.state().tokens_for_sale.iter().count(), 1, "Expected exactly one token listed for sale");
    }

    #[concordium_test]
    fn test_withdraw() {
        let mut ctx = TestReceiveContext::empty();
        ctx.set_owner(OWNER);
        ctx.set_sender(OWNER_ADDR);

        let mut state_builder = TestStateBuilder::new();
        let mut host = TestHost::new(State::empty(&mut state_builder), state_builder);

        host.state_mut().tokens_for_sale.insert(TOKEN1_ID, TOKEN1_PRICE);

        let param = WithdrawParameter {
            token_id: TOKEN1_ID,
        };

        let param_with_sender: ParamWithSender<WithdrawParameter> = ParamWithSender {
            sender: OWNER_ADDR,
            params: param,
        };

        let param_bytes = to_bytes(&param_with_sender);
        ctx.set_parameter(&param_bytes);

        let result = marketplace_withdraw(&ctx, &mut host);
        claim!(result.is_ok(), "Withdraw results in rejection.");

        claim_eq!(host.state().tokens_for_sale.iter().count(), 0, "After withdraw there should be no tokens for sale.");
    }

    #[concordium_test]
    fn test_purchase() {
        let mut ctx = TestReceiveContext::empty();
        ctx.set_owner(OWNER);
        ctx.set_sender(OWNER_ADDR);

        let mut state_builder = TestStateBuilder::new();
        let mut host = TestHost::new(State::empty(&mut state_builder), state_builder);

        host.state_mut().tokens_for_sale.insert(TOKEN1_ID, TOKEN1_PRICE);

        host.setup_mock_entrypoint(NFT_CONTRACT, EntrypointName::new_unchecked("transfer").into(), MockFn::returning_ok(0));

        let param = PurchaseParameter {
            token_id: TOKEN1_ID,
            from: OWNER,
            to: RECEIVER,
            contract: NFT_CONTRACT,
        };

        let param_with_sender: ParamWithSender<PurchaseParameter> = ParamWithSender {
            sender: OWNER_ADDR,
            params: param,
        };

        let param_bytes = to_bytes(&param_with_sender);
        ctx.set_parameter(&param_bytes);

        let result = marketplace_purchase(&ctx, &mut host);
        claim!(result.is_ok(), "Purchase results in rejection");

        claim_eq!(host.state().tokens_for_sale.iter().count(), 0, "There should be no tokens for sale left");
    }

    #[concordium_test]
    fn test_view_tokens_for_sale() {
        let ctx = TestReceiveContext::empty();

        let mut state_builder = TestStateBuilder::new();
        let mut host = TestHost::new(State::empty(&mut state_builder), state_builder);

        host.state_mut().tokens_for_sale.insert(TOKEN1_ID, TOKEN1_PRICE);
        
        let result = marketplace_view_list_for_sale(&ctx, &host);

        let view = result.expect_report("View list for sale results in rejection.");
        claim_eq!(view.tokens, vec![ViewStateToken{ id: TOKEN1_ID, price: TOKEN1_PRICE}], "Results should contain TOKEN1.");
    }


    #[concordium_test]
    fn test_get_listed_for_sale() {
        let mut ctx = TestReceiveContext::empty();

        let mut state_builder = TestStateBuilder::new();
        let mut host = TestHost::new(State::empty(&mut state_builder), state_builder);

        host.state_mut().tokens_for_sale.insert(TOKEN1_ID, TOKEN1_PRICE);
        
        let param = GetListedParameter{
            token_ids: vec![TOKEN1_ID, TOKEN2_ID],
        };

        let param_with_sender: ParamWithSender<GetListedParameter> = ParamWithSender {
            sender: OWNER_ADDR,
            params: param,
        };

        let param_bytes = to_bytes(&param_with_sender);
        ctx.set_parameter(&param_bytes);

        let result = marketplace_get_listed_for_sale(&ctx, &host);

        let view = result.expect_report("Get listed for sale results in rejection.");
        claim_eq!(view.tokens, vec![ViewStateToken{ id: TOKEN1_ID, price: TOKEN1_PRICE}], "Results should contain only TOKEN1.");
    }
}