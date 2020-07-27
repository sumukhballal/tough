// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: MIT OR Apache-2.0
mod test_utils;
use rusoto_kms::KmsClient;
extern crate rusoto_mock;
use self::rusoto_mock::*;
use ring::rand::SystemRandom;
use rusoto_core::signature::SignedRequest;
use rusoto_core::{HttpDispatchError, Region};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use tough::key_source::KeySource;
use tough::schema::key::{Key, RsaKey, RsaScheme};

#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
struct PublicKeyResp {
    #[serde(rename = "PublicKey")]
    #[serde(
        deserialize_with = "::rusoto_core::serialization::SerdeBlob::deserialize_blob",
        serialize_with = "::rusoto_core::serialization::SerdeBlob::serialize_blob",
        default
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    public_key: bytes::Bytes,
}

#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
struct SignResp {
    #[serde(rename = "Signature")]
    #[serde(
        deserialize_with = "::rusoto_core::serialization::SerdeBlob::deserialize_blob",
        serialize_with = "::rusoto_core::serialization::SerdeBlob::serialize_blob",
        default
    )]
    #[serde(skip_serializing_if = "Option::is_none")]
    signature: bytes::Bytes,
}

#[derive(Default, Debug, Clone, PartialEq, Deserialize)]
struct CreateKeyResp {
    #[serde(rename = "KeyId")]
    #[serde(skip_serializing_if = "Option::is_none")]
    key_id: String,
}

#[test]
// Ensure public key is returned on calling tuf_key
fn check_tuf_key_success() {
    let input = "response_public_key.json";
    let key_id = String::from("alias/some_alias");
    let file = File::open(test_utils::test_data().join(input).to_str().unwrap()).unwrap();
    let reader = BufReader::new(file);
    let expected_json: PublicKeyResp = serde_json::from_reader(reader).unwrap();
    let expected_key = Key::Rsa {
        keyval: RsaKey {
            public: expected_json.public_key.to_vec().into(),
            _extra: HashMap::new(),
        },
        scheme: RsaScheme::RsassaPssSha256,
        _extra: HashMap::new(),
    };
    let mock = MockRequestDispatcher::default()
        .with_request_checker(|request: &SignedRequest| {
            assert!(request
                .headers
                .get("x-amz-target")
                .unwrap()
                .contains(&Vec::from("TrentService.GetPublicKey")));
        })
        .with_body(
            MockResponseReader::read_response(test_utils::test_data().to_str().unwrap(), input)
                .as_ref(),
        );
    let mock_client = KmsClient::new_with(mock, MockCredentialsProvider, Region::UsEast1);
    let kms_key = tough_kms::KmsKeySource {
        profile: None,
        key_id: key_id.clone(),
        client: Some(mock_client),
    };
    let sign = kms_key.as_sign().unwrap();
    let key = sign.tuf_key();
    assert!(matches!(key, Key::Rsa { .. }));
    assert_eq!(key, expected_key);
}

#[test]
// Ensure message signature is returned on calling sign
fn check_sign_success() {
    let resp_public_key = "response_public_key.json";
    let resp_signature = "response_signature.json";
    let file = File::open(
        test_utils::test_data()
            .join(resp_signature)
            .to_str()
            .unwrap(),
    )
    .unwrap();
    let reader = BufReader::new(file);
    let expected_json: SignResp = serde_json::from_reader(reader).unwrap();
    let expected_signature = expected_json.signature.to_vec();
    let mock = MultipleMockRequestDispatcher::new(vec![
        MockRequestDispatcher::with_status(200)
            .with_request_checker(|request: &SignedRequest| {
                assert!(request
                    .headers
                    .get("x-amz-target")
                    .unwrap()
                    .contains(&Vec::from("TrentService.GetPublicKey")));
            })
            .with_body(
                MockResponseReader::read_response(
                    test_utils::test_data().to_str().unwrap(),
                    resp_public_key,
                )
                .as_ref(),
            ),
        MockRequestDispatcher::with_status(200)
            .with_request_checker(|request: &SignedRequest| {
                assert!(request
                    .headers
                    .get("x-amz-target")
                    .unwrap()
                    .contains(&Vec::from("TrentService.Sign")));
            })
            .with_body(
                MockResponseReader::read_response(
                    test_utils::test_data().to_str().unwrap(),
                    resp_signature,
                )
                .as_ref(),
            ),
    ]);
    let mock_client = KmsClient::new_with(mock, MockCredentialsProvider, Region::UsEast1);
    let kms_key = tough_kms::KmsKeySource {
        profile: None,
        key_id: String::from("alias/some_alias"),
        client: Some(mock_client),
    };
    let rng = SystemRandom::new();
    let kms_sign = kms_key.as_sign().unwrap();
    let signature = kms_sign
        .sign("Some message to sign".as_bytes(), &rng)
        .unwrap();
    assert_eq!(signature, expected_signature);
}

#[test]
// Ensure call to tuf_key fails when public key is not available
fn check_public_key_failure() {
    let error_msg = String::from("Some error message from KMS");
    let mock =
        MockRequestDispatcher::with_dispatch_error(HttpDispatchError::new(error_msg.clone()));
    let client = KmsClient::new_with(mock, MockCredentialsProvider, Region::UsEast1);
    let key_id = String::from("alias/some_alias");
    let kms_key = tough_kms::KmsKeySource {
        profile: None,
        key_id: key_id.clone(),
        client: Some(client),
    };
    let result = kms_key.as_sign();
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert_eq!(
        format!(
            "Failed to get public key for aws-kms:///{} : {}",
            key_id.clone(),
            error_msg.clone()
        ),
        err.to_string()
    );
}

#[test]
// Ensure sign error when Kms fails to sign message.
fn check_sign_request_failure() {
    let error_msg = String::from("Some error message from KMS");
    let resp_public_key = "response_public_key.json";
    let key_id = String::from("alias/some_alias");
    let mock = MultipleMockRequestDispatcher::new(vec![
        MockRequestDispatcher::with_status(200)
            .with_request_checker(|request: &SignedRequest| {
                assert!(request
                    .headers
                    .get("x-amz-target")
                    .unwrap()
                    .contains(&Vec::from("TrentService.GetPublicKey")));
            })
            .with_body(
                MockResponseReader::read_response(
                    test_utils::test_data().to_str().unwrap(),
                    resp_public_key,
                )
                .as_ref(),
            ),
        MockRequestDispatcher::with_dispatch_error(HttpDispatchError::new(error_msg.clone()))
            .with_request_checker(|request: &SignedRequest| {
                assert!(request
                    .headers
                    .get("x-amz-target")
                    .unwrap()
                    .contains(&Vec::from("TrentService.Sign")));
            }),
    ]);
    let mock_client = KmsClient::new_with(mock, MockCredentialsProvider, Region::UsEast1);
    let kms_key = tough_kms::KmsKeySource {
        profile: None,
        key_id: key_id.clone(),
        client: Some(mock_client),
    };
    let rng = SystemRandom::new();
    let kms_sign = kms_key.as_sign().unwrap();
    let result = kms_sign.sign("Some message to sign".as_bytes(), &rng);
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert_eq!(
        format!(
            "Error while signing message for aws-kms:///{} : {}",
            key_id.clone(),
            error_msg.clone()
        ),
        err.to_string()
    );
}

#[test]
// Ensure sign error when Kms returns empty signature.
fn check_signature_failure() {
    let resp_public_key = "response_public_key.json";
    let resp_signature = "response_signature_empty.json";
    let key_id = String::from("alias/some_alias");
    let mock = MultipleMockRequestDispatcher::new(vec![
        MockRequestDispatcher::with_status(200)
            .with_request_checker(|request: &SignedRequest| {
                assert!(request
                    .headers
                    .get("x-amz-target")
                    .unwrap()
                    .contains(&Vec::from("TrentService.GetPublicKey")));
            })
            .with_body(
                MockResponseReader::read_response(
                    test_utils::test_data().to_str().unwrap(),
                    resp_public_key,
                )
                .as_ref(),
            ),
        MockRequestDispatcher::with_status(200)
            .with_request_checker(|request: &SignedRequest| {
                assert!(request
                    .headers
                    .get("x-amz-target")
                    .unwrap()
                    .contains(&Vec::from("TrentService.Sign")));
            })
            .with_body(
                MockResponseReader::read_response(
                    test_utils::test_data().to_str().unwrap(),
                    resp_signature,
                )
                .as_ref(),
            ),
    ]);
    let mock_client = KmsClient::new_with(mock, MockCredentialsProvider, Region::UsEast1);
    let kms_key = tough_kms::KmsKeySource {
        profile: None,
        key_id: key_id.clone(),
        client: Some(mock_client),
    };
    let rng = SystemRandom::new();
    let kms_sign = kms_key.as_sign().unwrap();
    let result = kms_sign.sign("Some message to sign".as_bytes(), &rng);
    assert!(result.is_err());
    let err = result.err().unwrap();
    assert_eq!(
        format!("Empty signature returned by AWS KMS"),
        err.to_string()
    );
}
