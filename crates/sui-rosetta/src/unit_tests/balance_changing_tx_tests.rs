// Copyright (c) Mysten Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::str::FromStr;

use anyhow::anyhow;
use move_core_types::identifier::Identifier;
use rand::seq::{IteratorRandom, SliceRandom};
use serde_json::json;
use signature::rand_core::OsRng;
use sui_json_rpc_types::SuiTransactionResponseOptions;
use sui_types::programmable_transaction_builder::ProgrammableTransactionBuilder;

use crate::operations::Operations;
use shared_crypto::intent::Intent;
use sui_framework_build::compiled_package::BuildConfig;
use sui_json_rpc_types::{ObjectChange, SuiObjectRef};
use sui_keys::keystore::AccountKeystore;
use sui_keys::keystore::Keystore;
use sui_sdk::rpc_types::{
    OwnedObjectRef, SuiData, SuiExecutionStatus, SuiObjectDataOptions, SuiTransactionEffectsAPI,
    SuiTransactionResponse,
};
use sui_sdk::SuiClient;
use sui_types::base_types::{ObjectID, ObjectRef, SuiAddress};
use sui_types::gas_coin::GasCoin;
use sui_types::messages::{
    CallArg, ExecuteTransactionRequestType, InputObjectKind, ObjectArg, ProgrammableTransaction,
    Transaction, TransactionData, TransactionDataAPI, TransactionKind, DUMMY_GAS_PRICE,
};
use test_utils::network::TestClusterBuilder;

use crate::state::extract_balance_changes_from_ops;
use crate::types::ConstructionMetadata;

#[tokio::test]
async fn test_transfer_sui() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Transfer Sui
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient = get_random_address(&network.accounts, vec![sender]);
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder.transfer_sui(recipient, Some(50000));
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient],
        sender,
        pt,
        vec![],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_transfer_sui_whole_coin() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test transfer sui whole coin
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient = get_random_address(&network.accounts, vec![sender]);
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder.transfer_sui(recipient, None);
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient],
        sender,
        pt,
        vec![],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_transfer_object() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test transfer object
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient = get_random_address(&network.accounts, vec![sender]);
    let object_ref = get_random_sui(&client, sender, vec![]).await;
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder.transfer_object(recipient, object_ref).unwrap();
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient],
        sender,
        pt,
        vec![],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_publish_and_move_call() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test publish
    let sender = get_random_address(&network.accounts, vec![]);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../sui_programmability/examples/fungible_tokens");
    let package = sui_framework::build_move_package(&path, BuildConfig::new_for_testing()).unwrap();
    let compiled_module = package
        .get_modules()
        .map(|m| {
            let mut module_bytes = Vec::new();
            m.serialize(&mut module_bytes).unwrap();
            module_bytes
        })
        .collect::<Vec<_>>();

    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder.publish_immutable(compiled_module);
        builder.finish()
    };
    let response =
        test_transaction(&client, keystore, vec![], sender, pt, vec![], 10000, false).await;
    let object_changes = response.object_changes.unwrap();

    // Test move call (reuse published module from above test)
    let package = object_changes
        .iter()
        .find_map(|change| {
            if let ObjectChange::Published { package_id, .. } = change {
                Some(package_id)
            } else {
                None
            }
        })
        .unwrap();

    // TODO: Improve tx response to make it easier to find objects.
    let treasury = find_module_object(&object_changes, "::TreasuryCap");
    let treasury = treasury.clone().reference.to_object_ref();
    let recipient = *network.accounts.choose(&mut OsRng::default()).unwrap();
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder
            .move_call(
                *package,
                Identifier::from_str("managed").unwrap(),
                Identifier::from_str("mint").unwrap(),
                vec![],
                vec![
                    CallArg::Object(ObjectArg::ImmOrOwnedObject(treasury)),
                    CallArg::Pure(bcs::to_bytes(&10000u64).unwrap()),
                    CallArg::Pure(bcs::to_bytes(&recipient).unwrap()),
                ],
            )
            .unwrap();
        builder.finish()
    };

    test_transaction(&client, keystore, vec![], sender, pt, vec![], 10000, false).await;
}

#[tokio::test]
async fn test_split_coin() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test spilt coin
    let sender = get_random_address(&network.accounts, vec![]);
    let coin = get_random_sui(&client, sender, vec![]).await;
    let tx = client
        .transaction_builder()
        .split_coin(sender, coin.0, vec![100000], None, 10000)
        .await
        .unwrap();
    let pt = match tx.into_kind() {
        TransactionKind::ProgrammableTransaction(pt) => pt,
        _ => unreachable!(),
    };
    test_transaction(&client, keystore, vec![], sender, pt, vec![], 10000, false).await;
}

#[tokio::test]
async fn test_merge_coin() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test merge coin
    let sender = get_random_address(&network.accounts, vec![]);
    let coin = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin.0]).await;
    let tx = client
        .transaction_builder()
        .merge_coins(sender, coin.0, coin2.0, None, 10000)
        .await
        .unwrap();
    let pt = match tx.into_kind() {
        TransactionKind::ProgrammableTransaction(pt) => pt,
        _ => unreachable!(),
    };
    test_transaction(&client, keystore, vec![], sender, pt, vec![], 10000, false).await;
}

#[tokio::test]
async fn test_pay() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Pay
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient = get_random_address(&network.accounts, vec![sender]);
    let coin = get_random_sui(&client, sender, vec![]).await;
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder
            .pay(vec![coin], vec![recipient], vec![100000])
            .unwrap();
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient],
        sender,
        pt,
        vec![],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_pay_multiple_coin_multiple_recipient() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Pay multiple coin multiple recipient
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient1 = get_random_address(&network.accounts, vec![sender]);
    let recipient2 = get_random_address(&network.accounts, vec![sender, recipient1]);
    let coin1 = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin1.0]).await;
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder
            .pay(
                vec![coin1, coin2],
                vec![recipient1, recipient2],
                vec![100000, 200000],
            )
            .unwrap();
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient1, recipient2],
        sender,
        pt,
        vec![],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_pay_sui_multiple_coin_same_recipient() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Pay multiple coin same recipient
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient1 = get_random_address(&network.accounts, vec![sender]);
    let coin1 = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin1.0]).await;
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder
            .pay_sui(
                vec![recipient1, recipient1, recipient1],
                vec![100000, 100000, 100000],
            )
            .unwrap();
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient1],
        sender,
        pt,
        vec![coin1, coin2],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_pay_sui() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Pay Sui
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient1 = get_random_address(&network.accounts, vec![sender]);
    let recipient2 = get_random_address(&network.accounts, vec![sender, recipient1]);
    let coin1 = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin1.0]).await;
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder
            .pay_sui(vec![recipient1, recipient2], vec![1000000, 2000000])
            .unwrap();
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient1, recipient2],
        sender,
        pt,
        vec![coin1, coin2],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_failed_pay_sui() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test failed Pay Sui
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient1 = get_random_address(&network.accounts, vec![sender]);
    let recipient2 = get_random_address(&network.accounts, vec![sender, recipient1]);
    let coin1 = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin1.0]).await;
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder
            .pay_sui(vec![recipient1, recipient2], vec![1000000, 2000000])
            .unwrap();
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![],
        sender,
        pt,
        vec![coin1, coin2],
        110,
        true,
    )
    .await;
}

#[tokio::test]
async fn test_stake_sui() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Delegate Sui
    let sender = get_random_address(&network.accounts, vec![]);
    let coin1 = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin1.0]).await;
    let validator = client
        .governance_api()
        .get_latest_sui_system_state()
        .await
        .unwrap()
        .active_validators[0]
        .sui_address;
    let tx = client
        .transaction_builder()
        .request_add_stake(
            sender,
            vec![coin1.0, coin2.0],
            Some(1000000),
            validator,
            None,
            100000,
        )
        .await
        .unwrap();
    let pt = match tx.into_kind() {
        TransactionKind::ProgrammableTransaction(pt) => pt,
        _ => unreachable!(),
    };
    test_transaction(&client, keystore, vec![], sender, pt, vec![], 10000, false).await;
}

#[tokio::test]
async fn test_stake_sui_with_none_amount() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Staking Sui
    let sender = get_random_address(&network.accounts, vec![]);
    let coin1 = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin1.0]).await;
    let validator = client
        .governance_api()
        .get_latest_sui_system_state()
        .await
        .unwrap()
        .active_validators[0]
        .sui_address;
    let tx = client
        .transaction_builder()
        .request_add_stake(
            sender,
            vec![coin1.0, coin2.0],
            None,
            validator,
            None,
            100000,
        )
        .await
        .unwrap();
    let pt = match tx.into_kind() {
        TransactionKind::ProgrammableTransaction(pt) => pt,
        _ => unreachable!(),
    };
    test_transaction(&client, keystore, vec![], sender, pt, vec![], 10000, false).await;
}

#[tokio::test]
async fn test_pay_all_sui() {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let keystore = &network.wallet.config.keystore;

    // Test Pay All Sui
    let sender = get_random_address(&network.accounts, vec![]);
    let recipient = get_random_address(&network.accounts, vec![sender]);
    let coin1 = get_random_sui(&client, sender, vec![]).await;
    let coin2 = get_random_sui(&client, sender, vec![coin1.0]).await;
    let pt = {
        let mut builder = ProgrammableTransactionBuilder::new();
        builder.pay_all_sui(recipient);
        builder.finish()
    };
    test_transaction(
        &client,
        keystore,
        vec![recipient],
        sender,
        pt,
        vec![coin1, coin2],
        10000,
        false,
    )
    .await;
}

#[tokio::test]
async fn test_delegation_parsing() -> Result<(), anyhow::Error> {
    let network = TestClusterBuilder::new().build().await.unwrap();
    let client = network.wallet.get_client().await.unwrap();
    let sender = get_random_address(&network.accounts, vec![]);
    let gas = get_random_sui(&client, sender, vec![]).await;
    let validator = client
        .governance_api()
        .get_latest_sui_system_state()
        .await
        .unwrap()
        .active_validators[0]
        .sui_address;

    let ops: Operations = serde_json::from_value(json!(
        [{
            "operation_identifier":{"index":0},
            "type":"Stake",
            "account": { "address" : sender.to_string() },
            "amount" : { "value": "-100000" , "currency": { "symbol": "SUI", "decimals": 9}},
            "metadata": { "Stake" : {"validator": validator.to_string()} }
        }]
    ))
    .unwrap();
    let metadata = ConstructionMetadata {
        sender,
        coins: vec![gas],
        objects: vec![],
        total_coin_value: 0,
        gas_price: client.read_api().get_reference_gas_price().await?,
        budget: 10000,
    };
    let parsed_data = ops.clone().into_internal()?.try_into_data(metadata)?;
    assert_eq!(ops, Operations::try_from(parsed_data)?);

    Ok(())
}

fn find_module_object(changes: &[ObjectChange], object_type_name: &str) -> OwnedObjectRef {
    let mut results: Vec<_> = changes
        .iter()
        .filter_map(|change| {
            if let ObjectChange::Created {
                object_id,
                object_type,
                owner,
                version,
                digest,
                ..
            } = change
            {
                if object_type.to_string().contains(object_type_name) {
                    return Some(OwnedObjectRef {
                        owner: *owner,
                        reference: SuiObjectRef {
                            object_id: *object_id,
                            version: *version,
                            digest: *digest,
                        },
                    });
                }
            };
            None
        })
        .collect();
    // Check that there is only one object found, and hence no ambiguity.
    assert_eq!(results.len(), 1);
    results.pop().unwrap()
}

// Record current Sui balance of an address then execute the transaction,
// and compare the balance change reported by the event against the actual balance change.
async fn test_transaction(
    client: &SuiClient,
    keystore: &Keystore,
    addr_to_check: Vec<SuiAddress>,
    sender: SuiAddress,
    tx: ProgrammableTransaction,
    gas: Vec<ObjectRef>,
    budget: u64,
    expect_fail: bool,
) -> SuiTransactionResponse {
    let gas = if !gas.is_empty() {
        gas
    } else {
        let input_objects = tx
            .input_objects()
            .unwrap_or_default()
            .iter()
            .flat_map(|obj| {
                if let InputObjectKind::ImmOrOwnedMoveObject((id, ..)) = obj {
                    Some(*id)
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        vec![get_random_sui(client, sender, input_objects).await]
    };

    let data = TransactionData::new_with_gas_coins(
        TransactionKind::programmable(tx.clone()),
        sender,
        gas,
        budget,
        DUMMY_GAS_PRICE,
    );

    let signature = keystore
        .sign_secure(&data.sender(), &data, Intent::default())
        .unwrap();

    // Balance before execution
    let mut balances = BTreeMap::new();
    let mut addr_to_check = addr_to_check;
    addr_to_check.push(sender);
    for addr in addr_to_check {
        balances.insert(addr, get_balance(client, addr).await);
    }

    let response = client
        .quorum_driver()
        .execute_transaction(
            Transaction::from_data(data.clone(), Intent::default(), vec![signature])
                .verify()
                .unwrap(),
            SuiTransactionResponseOptions::full_content(),
            Some(ExecuteTransactionRequestType::WaitForLocalExecution),
        )
        .await
        .map_err(|e| anyhow!("TX execution failed for {data:#?}, error : {e}"))
        .unwrap();

    let effects = response.effects.as_ref().unwrap();

    if !expect_fail {
        assert_eq!(
            SuiExecutionStatus::Success,
            *effects.status(),
            "TX execution failed for {:#?}",
            data
        );
    } else {
        assert!(matches!(
            effects.status(),
            SuiExecutionStatus::Failure { .. }
        ));
    }

    let ops = response.clone().try_into().unwrap();
    let balances_from_ops = extract_balance_changes_from_ops(ops);

    // get actual balance changed after transaction
    let mut actual_balance_change = HashMap::new();
    for (addr, balance) in balances {
        let new_balance = get_balance(client, addr).await as i128;
        let balance_changed = new_balance - balance as i128;
        actual_balance_change.insert(addr, balance_changed);
    }
    assert_eq!(
        actual_balance_change, balances_from_ops,
        "balance check failed for tx: {}\neffect:{:#?}",
        tx, effects
    );
    response
}

async fn get_random_sui(
    client: &SuiClient,
    sender: SuiAddress,
    except: Vec<ObjectID>,
) -> ObjectRef {
    let coins = client
        .read_api()
        .get_owned_objects(
            sender,
            Some(SuiObjectDataOptions::full_content()),
            /* cursor */ None,
            /* limit */ None,
            /* at_checkpoint */ None,
        )
        .await
        .unwrap()
        .data;

    let coin_resp = coins
        .iter()
        .filter(|object| {
            let obj = object.object().unwrap();
            obj.is_gas_coin() && !except.contains(&obj.object_id)
        })
        .choose(&mut OsRng::default())
        .unwrap();

    let coin = coin_resp.object().unwrap();
    (coin.object_id, coin.version, coin.digest)
}

fn get_random_address(addresses: &[SuiAddress], except: Vec<SuiAddress>) -> SuiAddress {
    *addresses
        .iter()
        .filter(|addr| !except.contains(*addr))
        .choose(&mut OsRng::default())
        .unwrap()
}

async fn get_balance(client: &SuiClient, address: SuiAddress) -> u64 {
    let coins = client
        .read_api()
        .get_owned_objects(
            address,
            Some(SuiObjectDataOptions::full_content()),
            /* cursor */ None,
            /* limit */ None,
            /* at_checkpoint */ None,
        )
        .await
        .unwrap()
        .data;

    let mut balance = 0u64;
    for coin in coins {
        let obj = coin.into_object().unwrap();
        if obj.is_gas_coin() {
            let object = client
                .read_api()
                .get_object_with_options(obj.object_id, SuiObjectDataOptions::new().with_bcs())
                .await
                .unwrap();
            let coin: GasCoin = object
                .into_object()
                .unwrap()
                .bcs
                .unwrap()
                .try_as_move()
                .unwrap()
                .deserialize()
                .unwrap();
            balance += coin.value()
        }
    }
    balance
}
