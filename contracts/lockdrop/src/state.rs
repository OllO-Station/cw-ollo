use cosmwasm_std::{Addr, Deps, StdError, StdResult, Decimal256};
use cw_storage_plus::{Item, Map};

pub trait CompatibleLoader<K, R> {
    fn compat_load(&self, deps: Deps, key: K, generator: &Option<Addr>) -> StdResult<R>;

    fn compat_may_load(&self, deps: Deps, key: K, generator: &Option<Addr>) -> StdResult<Option<R>>;
}