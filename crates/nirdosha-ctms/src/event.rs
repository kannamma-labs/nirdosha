//! One canonical `TransactionEvent` shape for every source instrument
//! (card, transfer, cheque, cash, wallet, reversal, chargeback,
//! adjustment) -- the CTMS design note's own rule: "Do not let each
//! transaction type have a separate policy evaluator. Normalize them
//! first." Everything downstream (the velocity rule, the alert/case
//! gateways) only ever sees this one shape.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    Debit,
    Credit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    CardDebit,
    CardCredit,
    BankTransfer,
    Cheque,
    Cash,
    WalletMovement,
    Reversal,
    Chargeback,
    Adjustment,
}

/// The canonical shape. `partition`/`offset` and `ingestion_time_ms` are
/// not part of the wire payload -- they're stamped on by the stream
/// adapter from where it actually read the record, same as
/// `nirdosha_ingestion_topic_kafka::RawBatch`'s own offset convention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TransactionEvent {
    pub event_id: String,
    pub transaction_id: String,
    pub event_type: EventType,
    pub customer_id: String,
    pub account_id: String,
    pub direction: Direction,
    pub amount_minor: u64,
    pub currency: String,
    pub event_time_ms: u64,
    pub source_system: String,
    pub source_version: String,
    /// `event_id` of the transaction this one corrects/versions, if any.
    #[serde(default)]
    pub correction_of: Option<String>,
    pub jurisdiction: String,
    #[serde(skip_deserializing, default)]
    pub ingestion_time_ms: u64,
    #[serde(skip_deserializing, default)]
    pub partition: i32,
    #[serde(skip_deserializing, default)]
    pub offset: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizeError {
    /// The payload wasn't even well-formed JSON, or was missing/mistyped
    /// a required field -- `serde_json`'s own diagnostic, not swallowed.
    Malformed(String),
}

/// Deserializes and validates one raw wire payload into a canonical
/// [`TransactionEvent`], stamping the fields the stream adapter itself
/// owns (`partition`/`offset`/`ingestion_time_ms`) rather than trusting
/// the payload for them.
pub fn normalize(raw: &[u8], partition: i32, offset: i64, ingestion_time_ms: u64) -> Result<TransactionEvent, NormalizeError> {
    let mut event: TransactionEvent = serde_json::from_slice(raw).map_err(|e| NormalizeError::Malformed(e.to_string()))?;
    if event.event_id.trim().is_empty() {
        return Err(NormalizeError::Malformed("event_id must not be empty".to_string()));
    }
    if event.amount_minor == 0 {
        return Err(NormalizeError::Malformed("amount_minor must be greater than zero".to_string()));
    }
    event.partition = partition;
    event.offset = offset;
    event.ingestion_time_ms = ingestion_time_ms;
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_json() -> serde_json::Value {
        serde_json::json!({
            "event_id": "evt-1",
            "transaction_id": "txn-1",
            "event_type": "card_debit",
            "customer_id": "cust-1",
            "account_id": "acct-1",
            "direction": "debit",
            "amount_minor": 10000,
            "currency": "INR",
            "event_time_ms": 1_000,
            "source_system": "card-network-x",
            "source_version": "v1",
            "jurisdiction": "IN",
        })
    }

    #[test]
    fn normalizes_a_well_formed_event_and_stamps_adapter_owned_fields() {
        let event = normalize(valid_json().to_string().as_bytes(), 0, 42, 5_000).unwrap();
        assert_eq!(event.event_id, "evt-1");
        assert_eq!(event.partition, 0);
        assert_eq!(event.offset, 42);
        assert_eq!(event.ingestion_time_ms, 5_000);
        assert_eq!(event.correction_of, None);
    }

    #[test]
    fn rejects_malformed_json() {
        let err = normalize(b"{not json", 0, 0, 0).unwrap_err();
        assert!(matches!(err, NormalizeError::Malformed(_)));
    }

    #[test]
    fn rejects_a_payload_missing_a_required_field() {
        let mut value = valid_json();
        value.as_object_mut().unwrap().remove("amount_minor");
        let err = normalize(value.to_string().as_bytes(), 0, 0, 0).unwrap_err();
        assert!(matches!(err, NormalizeError::Malformed(_)));
    }

    #[test]
    fn rejects_a_zero_amount() {
        let mut value = valid_json();
        value["amount_minor"] = serde_json::json!(0);
        let err = normalize(value.to_string().as_bytes(), 0, 0, 0).unwrap_err();
        assert!(matches!(err, NormalizeError::Malformed(_)));
    }
}
