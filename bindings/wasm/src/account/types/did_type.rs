// Copyright 2020-2022 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use identity_iota::account_storage::DIDType;
use wasm_bindgen::prelude::*;

/// Supported types representing a DID that can be generated by the storage interface.
#[wasm_bindgen(js_name = DIDType)]
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
pub enum WasmDIDType {
  IotaDID,
}

impl From<WasmDIDType> for DIDType {
  fn from(other: WasmDIDType) -> Self {
    match other {
      WasmDIDType::IotaDID => DIDType::IotaDID,
    }
  }
}

impl From<DIDType> for WasmDIDType {
  fn from(other: DIDType) -> Self {
    match other {
      DIDType::IotaDID => WasmDIDType::IotaDID,
    }
  }
}
