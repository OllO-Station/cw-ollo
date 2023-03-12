use cosmwasm_std::{
    Addr, AllBalanceResponse, BankQuery, Coin, QuerierWrapper,
    QueryRequest, StdResult, Uint128,
};
use cw20::{
    BalanceResponse as Cw20BalanceResponse, Cw20QueryMsg
};

/// Returns a native token's balance for a specific account.
///
/// * **denom** specifies the denomination used to return the balance (e.g uluna).
pub fn query_balance(
    querier: &QuerierWrapper,
    account_addr: impl Into<String>,
    denom: impl Into<String>,
) -> StdResult<Uint128> {
    querier
        .query_balance(account_addr, denom)
        .map(|coin| coin.amount)
}

/// Returns the total balances for all coins at a specified account address.
///
/// * **account_addr** address for which we query balances.
pub fn query_all_balances(querier: &QuerierWrapper, account_addr: Addr) -> StdResult<Vec<Coin>> {
    let all_balances: AllBalanceResponse =
        querier.query(&QueryRequest::Bank(BankQuery::AllBalances {
            address: String::from(account_addr),
        }))?;
    Ok(all_balances.amount)
}

/// Returns a token balance for an account.
///
/// * **contract_addr** token contract for which we return a balance.
///
/// * **account_addr** account address for which we return a balance.
pub fn query_token_balance(
    querier: &QuerierWrapper,
    contract_addr: impl Into<String>,
    account_addr: impl Into<String>,
) -> StdResult<Uint128> {
    // load balance from the token contract
    let resp: Cw20BalanceResponse = querier
        .query_wasm_smart(
            contract_addr,
            &Cw20QueryMsg::Balance {
                address: account_addr.into(),
            },
        )
        .unwrap_or_else(|_| Cw20BalanceResponse {
            balance: Uint128::zero(),
        });

    Ok(resp.balance)
}