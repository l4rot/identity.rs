use anyhow::Context;
use identity_iota::{
  iota::{IotaClientExt, IotaDocument, IotaIdentityClientExt, NetworkName},
  verification::{jws::JwsAlgorithm, MethodScope},
};
use identity_storage::{key_id_storage::KeyIdMemstore, key_storage::JwkMemStore, JwkDocumentExt, Storage};
use iota_sdk::{
  client::{
    api::GetAddressesOptions,
    node_api::indexer::query_parameters::QueryParameter,
    secret::{stronghold::StrongholdSecretManager, SecretManager},
    Client, Password,
  },
  crypto::keys::bip39,
  types::block::{
    address::{Address, Bech32Address, Hrp},
    output::AliasOutputBuilder,
  },
};
use rand::{
  distributions::{Alphanumeric, DistString},
  thread_rng,
};
use std::{net::SocketAddr, path::PathBuf};
use tokio::{net::TcpListener, task::JoinHandle};
use tonic::transport::Uri;

pub type MemStorage = Storage<JwkMemStore, KeyIdMemstore>;

pub static API_ENDPOINT: &str = "http://localhost:14265";
pub static FAUCET_ENDPOINT: &str = "http://localhost:8091/api/enqueue";

#[derive(Debug)]
pub struct TestServer {
  client: Client,
  addr: SocketAddr,
  _handle: JoinHandle<Result<(), tonic::transport::Error>>,
}

impl TestServer {
  pub async fn new() -> Self {
    let listener = TcpListener::bind("127.0.0.1:0")
      .await
      .expect("Failed to bind to random OS's port");
    let addr = listener.local_addr().unwrap();

    let client: Client = Client::builder()
      .with_primary_node(API_ENDPOINT, None)
      .unwrap()
      .finish()
      .await
      .expect("Failed to connect to API's endpoint");

    let server = identity_grpc::server::GRpcServer::new(client.clone())
      .into_router()
      .serve_with_incoming(tokio_stream::wrappers::TcpListenerStream::new(listener));
    TestServer {
      _handle: tokio::spawn(server),
      addr,
      client,
    }
  }

  pub fn endpoint(&self) -> Uri {
    format!("https://{}", self.addr)
      .parse()
      .expect("Failed to parse server's URI")
  }

  pub fn client(&self) -> &Client {
    &self.client
  }
}

pub async fn create_did(
  client: &Client,
  secret_manager: &mut SecretManager,
  storage: &MemStorage,
) -> anyhow::Result<(Address, IotaDocument, String)> {
  let address: Address = get_address_with_funds(client, secret_manager, FAUCET_ENDPOINT)
    .await
    .context("failed to get address with funds")?;

  let network_name = client.network_name().await?;
  let (document, fragment): (IotaDocument, String) = create_did_document(&network_name, storage).await?;
  let alias_output = client.new_did_output(address, document, None).await?;

  let document: IotaDocument = client.publish_did_output(secret_manager, alias_output).await?;

  Ok((address, document, fragment))
}

/// Creates an example DID document with the given `network_name`.
///
/// Its functionality is equivalent to the "create DID" example
/// and exists for convenient calling from the other examples.
pub async fn create_did_document(
  network_name: &NetworkName,
  storage: &MemStorage,
) -> anyhow::Result<(IotaDocument, String)> {
  let mut document: IotaDocument = IotaDocument::new(network_name);

  let fragment: String = document
    .generate_method(
      storage,
      JwkMemStore::ED25519_KEY_TYPE,
      JwsAlgorithm::EdDSA,
      None,
      MethodScope::VerificationMethod,
    )
    .await?;

  Ok((document, fragment))
}

/// Generates an address from the given [`SecretManager`] and adds funds from the faucet.
pub async fn get_address_with_funds(
  client: &Client,
  stronghold: &SecretManager,
  faucet_endpoint: &str,
) -> anyhow::Result<Address> {
  let address = get_address(client, stronghold).await?;

  request_faucet_funds(client, address, faucet_endpoint)
    .await
    .context("failed to request faucet funds")?;

  Ok(*address)
}

/// Initializes the [`SecretManager`] with a new mnemonic, if necessary,
/// and generates an address from the given [`SecretManager`].
pub async fn get_address(client: &Client, secret_manager: &SecretManager) -> anyhow::Result<Bech32Address> {
  let random: [u8; 32] = rand::random();
  let mnemonic = bip39::wordlist::encode(random.as_ref(), &bip39::wordlist::ENGLISH)
    .map_err(|err| anyhow::anyhow!(format!("{err:?}")))?;

  if let SecretManager::Stronghold(ref stronghold) = secret_manager {
    match stronghold.store_mnemonic(mnemonic).await {
      Ok(()) => (),
      Err(iota_sdk::client::stronghold::Error::MnemonicAlreadyStored) => (),
      Err(err) => anyhow::bail!(err),
    }
  } else {
    anyhow::bail!("expected a `StrongholdSecretManager`");
  }

  let bech32_hrp: Hrp = client.get_bech32_hrp().await?;
  let address: Bech32Address = secret_manager
    .generate_ed25519_addresses(
      GetAddressesOptions::default()
        .with_range(0..1)
        .with_bech32_hrp(bech32_hrp),
    )
    .await?[0];

  Ok(address)
}

/// Requests funds from the faucet for the given `address`.
async fn request_faucet_funds(client: &Client, address: Bech32Address, faucet_endpoint: &str) -> anyhow::Result<()> {
  iota_sdk::client::request_funds_from_faucet(faucet_endpoint, &address).await?;

  tokio::time::timeout(std::time::Duration::from_secs(45), async {
    loop {
      tokio::time::sleep(std::time::Duration::from_secs(5)).await;

      let balance = get_address_balance(client, &address)
        .await
        .context("failed to get address balance")?;
      if balance > 0 {
        break;
      }
    }
    Ok::<(), anyhow::Error>(())
  })
  .await
  .context("maximum timeout exceeded")??;

  Ok(())
}

pub struct Entity {
  secret_manager: SecretManager,
  storage: MemStorage,
  did: Option<(Address, IotaDocument, String)>,
}

pub fn random_password(len: usize) -> Password {
  let mut rng = thread_rng();
  Alphanumeric.sample_string(&mut rng, len).into()
}

pub fn random_stronghold_path() -> PathBuf {
  let mut file = std::env::temp_dir();
  file.push("test_strongholds");
  file.push(rand::distributions::Alphanumeric.sample_string(&mut rand::thread_rng(), 32));
  file.set_extension("stronghold");
  file.to_owned()
}

impl Entity {
  pub fn new() -> Self {
    let secret_manager = SecretManager::Stronghold(
      StrongholdSecretManager::builder()
        .password(random_password(18))
        .build(random_stronghold_path())
        .expect("Failed to create temporary stronghold"),
    );
    let storage = MemStorage::new(JwkMemStore::new(), KeyIdMemstore::new());

    Self {
      secret_manager,
      storage,
      did: None,
    }
  }

  pub async fn create_did(&mut self, client: &Client) -> anyhow::Result<()> {
    self.did = Some(create_did(client, &mut self.secret_manager, &self.storage).await?);

    Ok(())
  }

  pub fn document(&self) -> Option<&IotaDocument> {
    self.did.as_ref().map(|(_, doc, _)| doc)
  }

  pub async fn update_document<F>(&mut self, client: &Client, f: F) -> anyhow::Result<()>
  where
    F: FnOnce(IotaDocument) -> Option<IotaDocument>,
  {
    let (address, doc, fragment) = self.did.take().context("Missing doc")?;
    let mut new_doc = f(doc.clone());
    if let Some(doc) = new_doc.take() {
      let alias_output = client.update_did_output(doc.clone()).await?;
      let rent_structure = client.get_rent_structure().await?;
      let alias_output = AliasOutputBuilder::from(&alias_output)
        .with_minimum_storage_deposit(rent_structure)
        .finish()?;

      new_doc = Some(client.publish_did_output(&self.secret_manager, alias_output).await?);
    }

    self.did = Some((address, new_doc.unwrap_or(doc), fragment));

    Ok(())
  }
}
/// Returns the balance of the given Bech32-encoded `address`.
async fn get_address_balance(client: &Client, address: &Bech32Address) -> anyhow::Result<u64> {
  let output_ids = client
    .basic_output_ids(vec![
      QueryParameter::Address(address.to_owned()),
      QueryParameter::HasExpiration(false),
      QueryParameter::HasTimelock(false),
      QueryParameter::HasStorageDepositReturn(false),
    ])
    .await?;

  let outputs = client.get_outputs(&output_ids).await?;

  let mut total_amount = 0;
  for output_response in outputs {
    total_amount += output_response.output().amount();
  }

  Ok(total_amount)
}
