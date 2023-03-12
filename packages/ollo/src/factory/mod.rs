use crate::query::{
    balance::{query_balance, query_token_balance},
    token::NATIVE_TOKEN_PRECISION,
};
use cosmwasm_schema::{cw_serde, QueryResponses};
use cosmwasm_std::{
    Api, BankMsg, Coin, Uint128, Uint256, Addr, Binary,
    to_binary, StdError, StdResult, QuerierWrapper, WasmMsg,
    CosmosMsg, Decimal, Decimal256, OverflowError, MessageInfo,
};
use cw20::{Cw20ExecuteMsg, Cw20QueryMsg, MinterResponse, TokenInfoResponse};
use cw_storage_plus::{Map, Item};
use std::fmt::{Display, Formatter, self};

pub const ROUTE: Map<(String, String), Vec<Addr>> = Map::new("routes");
pub const MINIMUM_LIQUIDITY_AMOUNT: Uint128 = Uint128::new(1_000);
const MAX_TOTAL_FEE_BPS: u16 = 10_000;
const MAX_MAKER_FEE_BPS: u16 = 10_000;

///
pub fn addr_validate_to_lower(api: &dyn Api, addr: impl Into<String>) -> StdResult<Addr> {
    let addr = addr.into();
    if addr.to_lowercase() != addr {
        return Err(StdError::generic_err(format!(
            "Address {} should be lowercase",
            addr
        )));
    }
    api.addr_validate(&addr)
}
/// This enum describes a Terra asset (native or CW20).
#[cw_serde]
pub struct Asset {
    /// Information about an asset stored in a [`AssetInfo`] struct
    pub info: AssetInfo,
    /// A token amount
    pub amount: Uint128,
}

/// This struct describes a Terra asset as decimal.
#[cw_serde]
pub struct DecimalAsset {
    pub info: AssetInfo,
    pub amount: Decimal256,
}

impl fmt::Display for Asset {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}{}", self.amount, self.info)
    }
}

impl Asset {
    /// Returns true if the token is native. Otherwise returns false.
    pub fn is_native_token(&self) -> bool {
        self.info.is_native_token()
    }

    /// Calculates and returns a tax for a chain's native token. For other tokens it returns zero.
    pub fn compute_tax(&self, _querier: &QuerierWrapper) -> StdResult<Uint128> {
        // tax rate in Terra is set to zero https://terrawiki.org/en/developers/tx-fees
        Ok(Uint128::zero())
    }

    /// Calculates and returns a deducted tax for transferring the native token from the chain. For other tokens it returns an [`Err`].
    pub fn deduct_tax(&self, querier: &QuerierWrapper) -> StdResult<Coin> {
        if let AssetInfo::NativeToken { denom } = &self.info {
            Ok(Coin {
                denom: denom.to_string(),
                amount: self.amount.checked_sub(self.compute_tax(querier)?)?,
            })
        } else {
            Err(StdError::generic_err("cannot deduct tax from token asset"))
        }
    }

    /// For native tokens of type [`AssetInfo`] uses the default method [`BankMsg::Send`] to send a token amount to a recipient.
    /// Before the token is sent, we need to deduct a tax.
    ///
    /// For a token of type [`AssetInfo`] we use the default method [`Cw20ExecuteMsg::Transfer`] and so there's no need to deduct any other tax.
    pub fn into_msg(
        self,
        querier: &QuerierWrapper,
        recipient: impl Into<String>,
    ) -> StdResult<CosmosMsg> {
        let recipient = recipient.into();
        match &self.info {
            AssetInfo::Token { contract_addr } => Ok(CosmosMsg::Wasm(WasmMsg::Execute {
                contract_addr: contract_addr.to_string(),
                msg: to_binary(&Cw20ExecuteMsg::Transfer {
                    recipient,
                    amount: self.amount,
                })?,
                funds: vec![],
            })),
            AssetInfo::NativeToken { .. } => Ok(CosmosMsg::Bank(BankMsg::Send {
                to_address: recipient,
                amount: vec![self.deduct_tax(querier)?],
            })),
        }
    }

    /// Validates an amount of native tokens being sent.
    pub fn assert_sent_native_token_balance(&self, message_info: &MessageInfo) -> StdResult<()> {
        if let AssetInfo::NativeToken { denom } = &self.info {
            match message_info.funds.iter().find(|x| x.denom == *denom) {
                Some(coin) => {
                    if self.amount == coin.amount {
                        Ok(())
                    } else {
                        Err(StdError::generic_err("Native token balance mismatch between the argument and the transferred"))
                    }
                }
                None => {
                    if self.amount.is_zero() {
                        Ok(())
                    } else {
                        Err(StdError::generic_err("Native token balance mismatch between the argument and the transferred"))
                    }
                }
            }
        } else {
            Ok(())
        }
    }

    pub fn to_decimal_asset(&self, precision: impl Into<u32>) -> StdResult<DecimalAsset> {
        Ok(DecimalAsset {
            info: self.info.clone(),
            amount: Decimal256::from_atomics(self.amount, precision.into()).unwrap_or_default(),
        })
    }
}

/// This enum describes available Token types.
/// ## Examples
/// ```
/// # use cosmwasm_std::Addr;
/// # use astroport::asset::AssetInfo::{NativeToken, Token};
/// Token { contract_addr: Addr::unchecked("stake...") };
/// NativeToken { denom: String::from("uluna") };
/// ```
#[cw_serde]
#[derive(Hash, Eq)]
pub enum AssetInfo {
    /// Non-native Token
    Token { contract_addr: Addr },
    /// Native token
    NativeToken { denom: String },
}

impl fmt::Display for AssetInfo {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            AssetInfo::NativeToken { denom } => write!(f, "{}", denom),
            AssetInfo::Token { contract_addr } => write!(f, "{}", contract_addr),
        }
    }
}

impl AssetInfo {
    /// Returns true if the caller is a native token. Otherwise returns false.
    pub fn is_native_token(&self) -> bool {
        match self {
            AssetInfo::NativeToken { .. } => true,
            AssetInfo::Token { .. } => false,
        }
    }

    /// Returns the balance of token in a pool.
    ///
    /// * **pool_addr** is the address of the contract whose token balance we check.
    pub fn query_pool(
        &self,
        querier: &QuerierWrapper,
        pool_addr: impl Into<String>,
    ) -> StdResult<Uint128> {
        match self {
            AssetInfo::Token { contract_addr, .. } => {
                query_token_balance(querier, contract_addr, pool_addr)
            }
            AssetInfo::NativeToken { denom } => query_balance(querier, pool_addr, denom),
        }
    }

    /// Returns the number of decimals that a token has.
    pub fn decimals(&self, querier: &QuerierWrapper) -> StdResult<u8> {
        let decimals = match &self {
            AssetInfo::NativeToken { .. } => NATIVE_TOKEN_PRECISION,
            AssetInfo::Token { contract_addr } => {
                let res: TokenInfoResponse =
                    querier.query_wasm_smart(contract_addr, &Cw20QueryMsg::TokenInfo {})?;

                res.decimals
            }
        };

        Ok(decimals)
    }

    /// Returns **true** if the calling token is the same as the token specified in the input parameters.
    /// Otherwise returns **false**.
    pub fn equal(&self, asset: &AssetInfo) -> bool {
        match (self, asset) {
            (AssetInfo::NativeToken { denom }, AssetInfo::NativeToken { denom: other_denom }) => {
                denom == other_denom
            }
            (
                AssetInfo::Token { contract_addr },
                AssetInfo::Token {
                    contract_addr: other_contract_addr,
                },
            ) => contract_addr == other_contract_addr,
            _ => false,
        }
    }

    /// If the caller object is a native token of type [`AssetInfo`] then his `denom` field converts to a byte string.
    ///
    /// If the caller object is a token of type [`AssetInfo`] then its `contract_addr` field converts to a byte string.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            AssetInfo::NativeToken { denom } => denom.as_bytes(),
            AssetInfo::Token { contract_addr } => contract_addr.as_bytes(),
        }
    }

    /// Checks that the tokens' denom or contract addr is lowercased and valid.
    pub fn check(&self, api: &dyn Api) -> StdResult<()> {
        match self {
            AssetInfo::Token { contract_addr } => {
                addr_validate_to_lower(api, contract_addr)?;
            }
            AssetInfo::NativeToken { denom } => {
                if !denom.starts_with("ibc/") && denom != &denom.to_lowercase() {
                    return Err(StdError::generic_err(format!(
                        "Non-IBC token denom {} should be lowercase",
                        denom
                    )));
                }
            }
        }
        Ok(())
    }
}

/// This structure stores the main parameters for an Astroport pair
#[cw_serde]
pub struct PairInfo {
    /// Asset information for the assets in the pool
    pub asset_infos: Vec<AssetInfo>,
    /// Pair contract address
    pub contract_addr: Addr,
    /// Pair LP token address
    pub liquidity_token: Addr,
    /// The pool type (xyk, stableswap etc) available in [`PairType`]
    pub pair_type: PairType,
}

impl PairInfo {
    /// Returns the balance for each asset in the pool.
    ///
    /// * **contract_addr** is pair's pool address.
    pub fn query_pools(
        &self,
        querier: &QuerierWrapper,
        contract_addr: impl Into<String>,
    ) -> StdResult<Vec<Asset>> {
        let contract_addr = contract_addr.into();
        self.asset_infos
            .iter()
            .map(|asset_info| {
                Ok(Asset {
                    info: asset_info.clone(),
                    amount: asset_info.query_pool(querier, &contract_addr)?,
                })
            })
            .collect()
    }

    /// Returns the balance for each asset in the pool in decimal.
    ///
    /// * **contract_addr** is pair's pool address.
    pub fn query_pools_decimal(
        &self,
        querier: &QuerierWrapper,
        contract_addr: impl Into<String>,
    ) -> StdResult<Vec<DecimalAsset>> {
        let contract_addr = contract_addr.into();
        self.asset_infos
            .iter()
            .map(|asset_info| {
                Ok(DecimalAsset {
                    info: asset_info.clone(),
                    amount: Decimal256::from_atomics(
                        asset_info.query_pool(querier, &contract_addr)?,
                        asset_info.decimals(querier)?.into(),
                    )
                    .map_err(|_| StdError::generic_err("Decimal256RangeExceeded"))?,
                })
            })
            .collect()
    }
}

/// Available pair/liquidity pool types for pairs
#[cw_serde]
pub enum PairType {
    /// Stable swap pool type
    Stable {},
    /// Ranged pool type
    Ranged {},
    /// Const product, xy = k
    ConstProduct {},
    /// Custom pool type
    Custom(String),
}

impl Display for PairType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            PairType::Stable { .. } => write!(f, "Stable"),
            PairType::ConstProduct { .. } => write!(f, "ConstProduct"),
            PairType::Ranged { .. } => write!(f, "Ranged"),
            PairType::Custom(name) => write!(f, "{}", name),
        }
    }
}

///
#[cw_serde]
pub struct PairConfig {
    /// Contract id
    pub contract_id: u64,
    /// Pair type
    pub pair_type: PairType,
    ///
    pub total_fee_bps: u16,
    ///
    pub maker_fee_bps: u16,
    ///
    pub is_disabled: bool,
    ///
    pub is_generator_disabled: bool,
}

impl PairConfig {
    ///
    pub fn valid_fee_bps(&self) -> bool {
        self.total_fee_bps <= MAX_TOTAL_FEE_BPS
            && self.maker_fee_bps <= MAX_MAKER_FEE_BPS
    }
}

/// A custom struct for each query response that returns general contract settings/configs.
#[cw_serde]
pub struct ConfigResponse {
    /// Addres of owner that is allowed to change contract parameters
    pub owner: Addr,
    /// IDs of contracts which are allowed to create pairs
    pub pair_configs: Vec<PairConfig>,
    /// CW20 token contract code identifier
    pub token_code_id: u64,
    /// Address of contract to send governance fees to (the Maker)
    pub fee_address: Option<Addr>,
    /// Address of contract used to auto_stake LP tokens for Astroport pairs that are incentivized
    pub generator_address: Option<Addr>,
    /// CW1 whitelist contract code id used to store 3rd party rewards for staking Astroport LP tokens
    pub whitelist_code_id: u64,
}

/// This structure stores the parameters used in a migration message.
#[cw_serde]
pub struct MigrateMsg {
    pub params: Binary,
}

/// A custom struct for each query response that returns an array of objects of type [`PairInfo`].
#[cw_serde]
pub struct PairsResponse {
    /// Arrays of structs containing information about multiple pairs
    pub pairs: Vec<PairInfo>,
}

/// A custom struct for each query response that returns an object of type [`FeeInfoResponse`].
#[cw_serde]
pub struct FeeInfoResponse {
    /// Contract address to send governance fees to
    pub fee_address: Option<Addr>,
    /// Total amount of fees (in bps) charged on a swap
    pub total_fee_bps: u16,
    /// Amount of fees (in bps) sent to the Maker contract
    pub maker_fee_bps: u16,
}

/// This is an enum used for setting and removing a contract address.
#[cw_serde]
pub enum UpdateAddr {
    /// Sets a new contract address.
    Set(String),
    /// Removes a contract address.
    Remove {},
}

///
#[cw_serde]
pub struct InstantiateMsg {
    ///
    pub pair_configs: Vec<PairConfig>,
    ///
    pub token_code_id: u64,
    ///
    pub fee_address: Option<String>,
    ///
    pub generator_address: Option<String>,
    ///
    pub owner: String,
    ///
    pub whitelist_code_id: u64
}

///
#[cw_serde]
pub enum ExecuteMsg {
    ///
    UpdateConfig {
        ///
        token_code_id: Option<u64>,
        ///
        fee_address: Option<String>,
        ///
        generator_address: Option<String>,
        ///
        whitelist_code_id: Option<u64>,
    },
    ///
    UpdatePairConfig {
        ///
        config: PairConfig,
    },
    ///
    CreatePair {
        ///
        pair_type: PairType,
        ///
        asset_infos: Vec<AssetInfo>,
        ///
        init_params: Option<Binary>,
    },
    ///
    Deregister {
        ///
        asset_infos: Vec<AssetInfo>,
    },
    ///
    ProposeNewOwner {
        ///
        owner: String,
        ///
        expires_in: u64,
    },
    ///
    CancelOwnershipProposal {},
    ///
    ClaimOwnership {},
    ///
    MarkAsMigrated {
        ///
        pairs: Vec<String>,
    }
}

///
#[cw_serde]
#[derive(QueryResponses)]
pub enum QueryMsg {
    /// Config returns contract settings specified in the custom [`ConfigResponse`] structure.
    #[returns(ConfigResponse)]
    Config {},
    /// Pair returns information about a specific pair according to the specified assets.
    #[returns(PairInfo)]
    Pair {
        /// The assets for which we return a pair
        asset_infos: Vec<AssetInfo>,
    },
    /// Pairs returns an array of pairs and their information according to the specified parameters in `start_after` and `limit` variables.
    #[returns(PairsResponse)]
    Pairs {
        /// The pair item to start reading from. It is an [`Option`] type that accepts [`AssetInfo`] elements.
        start_after: Option<Vec<AssetInfo>>,
        /// The number of pairs to read and return. It is an [`Option`] type.
        limit: Option<u32>,
    },
    /// FeeInfo returns fee parameters for a specific pair. The response is returned using a [`FeeInfoResponse`] structure
    #[returns(FeeInfoResponse)]
    FeeInfo {
        /// The pair type for which we return fee information. Pair type is a [`PairType`] struct
        pair_type: PairType,
    },
    /// Returns a vector that contains blacklisted pair types
    #[returns(Vec<PairType>)]
    BlacklistedPairTypes {},
    /// Returns a vector that contains pair addresses that are not migrated
    #[returns(Vec<Addr>)]
    PairsToMigrate {},
}
