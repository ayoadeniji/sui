// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::str::FromStr;
use sui_json_rpc_types::SuiObjectDataOptions;
use sui_sdk::types::base_types::SuiAddress;
use sui_sdk::SuiClientBuilder;

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    let sui = SuiClientBuilder::default()
        .build("https://fullnode.devnet.sui.io:443")
        .await?;
    let address = SuiAddress::from_str("0xec11cad080d0496a53bafcea629fcbcfff2a9866")?;
    let objects = sui
        .read_api()
        .get_owned_objects(
            address,
            Some(SuiObjectDataOptions::default()),
            None,
            None,
            None,
        )
        .await?;
    println!("{:?}", objects);
    Ok(())
}
