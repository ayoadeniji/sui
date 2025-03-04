// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0
use move_core_types::language_storage::TypeTag;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use serde_with::DisplayFromStr;

use sui_types::object::Owner;

#[serde_as]
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BalanceChange {
    /// Owner of the balance change
    pub owner: Owner,
    #[schemars(with = "String")]
    #[serde_as(as = "DisplayFromStr")]
    pub coin_type: TypeTag,
    /// The amount indicate the balance value changes,
    /// negative amount means spending coin value and positive means receiving coin value.
    pub amount: i128,
}
