#![cfg(feature = "files")]

mod common;

use crate::common::{evm_network_from_env, evm_wallet_from_env_or_default};
use autonomi::Client;
#[cfg(feature = "vault")]
use bytes::Bytes;
#[cfg(feature = "vault")]
use eyre::bail;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::test]
async fn file() -> Result<(), Box<dyn std::error::Error>> {
    common::enable_logging();

    let network = evm_network_from_env();
    let mut client = Client::connect(&[]).await.unwrap();
    let wallet = evm_wallet_from_env_or_default(network);

    let (root, addr) = client
        .upload_from_dir("tests/file/test_dir".into(), &wallet)
        .await?;

    sleep(Duration::from_secs(10)).await;

    let root_fetched = client.fetch_root(addr).await?;

    assert_eq!(
        root.map, root_fetched.map,
        "root fetched should match root put"
    );

    Ok(())
}

#[cfg(feature = "vault")]
#[tokio::test]
async fn file_into_vault() -> eyre::Result<()> {
    common::enable_logging();

    let network = evm_network_from_env();

    let mut client = Client::connect(&[])
        .await?
        .with_vault_entropy(Bytes::from("at least 32 bytes of entropy here"))?;

    let wallet = evm_wallet_from_env_or_default(network);

    let (root, addr) = client
        .upload_from_dir("tests/file/test_dir".into(), &wallet)
        .await?;
    sleep(Duration::from_secs(2)).await;

    let root_fetched = client.fetch_root(addr).await?;

    assert_eq!(
        root.map, root_fetched.map,
        "root fetched should match root put"
    );

    // now assert over the stored account packet
    let new_client = Client::connect(&[])
        .await?
        .with_vault_entropy(Bytes::from("at least 32 bytes of entropy here"))?;

    if let Some(ap) = new_client.fetch_and_decrypt_vault().await? {
        let ap_root_fetched = Client::deserialize_root(ap)?;

        assert_eq!(
            root.map, ap_root_fetched.map,
            "root fetched should match root put"
        );
    } else {
        bail!("No account packet found");
    }

    Ok(())
}
