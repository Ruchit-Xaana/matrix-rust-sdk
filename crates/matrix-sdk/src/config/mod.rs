// Copyright 2021 The Matrix.org Foundation C.I.C.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Configuration to change the behaviour of the [`Client`][crate::Client].

mod client;
mod request;
mod sync;

pub use client::ClientConfig;
pub use matrix_sdk_base::store::StoreConfig;
pub use request::RequestConfig;
pub use sync::SyncSettings;
