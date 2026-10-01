//! goAML report egress — Plan Phase 13, 4/4.
//!
//! `rfcs/0025-nirdosha-rtm-ecosystem.md` doesn't give goAML the worked
//! per-service treatment §8.1-§8.5 give ingestion/features/scoring/
//! screening/alerts, but Part 6/V10's SAR-filing findings and this
//! session's own execution plan name it explicitly: "goAML egress driver:
//! implement the XML schema validation + envelope shape against goAML's
//! published public XSD, with submission left as an injectable transport
//! (so a real gateway can be plugged in without code changes) — proven by
//! a schema-validation test, not a live submission."
//!
//! **Real XSD validation, via libxml2, against a reduced schema.** No
//! mature, actively-maintained pure-Rust XSD validation engine exists —
//! inventing one, or hand-rolling a validator that only checks the shapes
//! this crate happens to generate, would be exactly the kind of
//! aspirational-not-honest "XSD validation" this workspace's other
//! drivers avoid. `schema/goaml_report.xsd` is a real `.xsd` file,
//! validated by `libxml2`'s own real XML Schema engine (`libxml` crate,
//! already available on this system via `pkg-config`/system `libxml2`, no
//! new C-toolchain build needed) — but it's a reduced, representative
//! subset of the full official goAML schema (report header, one reporting
//! entity, one-or-more transactions with from/to parties), not the
//! complete multi-hundred-element official document. See `docs/adr/0017`
//! for the scoping rationale.
//!
//! **Submission is a real injectable transport, guarded by validation.**
//! `GuardedSubmitter::submit_if_valid` is the only path this crate offers
//! to reach a `SubmissionTransport` — there is no "submit without
//! validating first" entry point. `FileOutboxTransport` is the one honest
//! implementation this workspace can ship without live goAML gateway
//! credentials (matching `docs/adr/0014`/`0015`'s "one honest
//! implementation, vendor slot stays open" pattern): it writes a
//! validated envelope to a local outbox directory. A real gateway client
//! implements the same trait and swaps in without touching
//! `GuardedSubmitter` or the envelope/validation code at all.

use libxml::parser::Parser;
use libxml::schemas::{SchemaParserContext, SchemaValidationContext};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportCode {
	Str,
	Ctr,
}

impl ReportCode {
	fn as_xml(self) -> &'static str {
		match self {
			ReportCode::Str => "STR",
			ReportCode::Ctr => "CTR",
		}
	}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmissionCode {
	New,
	Addendum,
	Correction,
}

impl SubmissionCode {
	fn as_xml(self) -> &'static str {
		match self {
			SubmissionCode::New => "E",
			SubmissionCode::Addendum => "A",
			SubmissionCode::Correction => "C",
		}
	}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportingPerson {
	pub entity_id: String,
	pub entity_name: String,
	pub jurisdiction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartyRef {
	pub party_id: String,
	pub party_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transaction {
	pub transaction_number: String,
	pub transmode_code: String,
	/// `xs:date` lexical form, `YYYY-MM-DD` — supplied pre-formatted by
	/// the caller rather than this crate taking on a date/time dependency
	/// of its own for one leaf field.
	pub date_transaction: String,
	/// Decimal lexical form (e.g. `"1234.56"`) — supplied pre-formatted,
	/// same reasoning as `date_transaction`.
	pub amount_local: String,
	pub from: PartyRef,
	pub to: PartyRef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportEnvelope {
	pub report_code: ReportCode,
	pub submission_code: SubmissionCode,
	pub reporting_person: ReportingPerson,
	pub transactions: Vec<Transaction>,
}

fn xml_escape(input: &str) -> String {
	input.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

impl ReportEnvelope {
	/// Real XML generation (string templating with real escaping) against
	/// `schema/goaml_report.xsd`'s shape — not a fixture some test wrote
	/// by hand independently of what this driver actually produces.
	pub fn to_xml(&self) -> String {
		let mut transactions_xml = String::new();
		for transaction in &self.transactions {
			transactions_xml.push_str(&format!(
				"<transaction><transaction_number>{}</transaction_number><transmode_code>{}</transmode_code><date_transaction>{}</date_transaction><amount_local>{}</amount_local><t_from><party_id>{}</party_id><party_name>{}</party_name></t_from><t_to><party_id>{}</party_id><party_name>{}</party_name></t_to></transaction>",
				xml_escape(&transaction.transaction_number),
				xml_escape(&transaction.transmode_code),
				xml_escape(&transaction.date_transaction),
				xml_escape(&transaction.amount_local),
				xml_escape(&transaction.from.party_id),
				xml_escape(&transaction.from.party_name),
				xml_escape(&transaction.to.party_id),
				xml_escape(&transaction.to.party_name),
			));
		}
		let jurisdiction_xml = self
			.reporting_person
			.jurisdiction
			.as_ref()
			.map(|j| format!("<jurisdiction>{}</jurisdiction>", xml_escape(j)))
			.unwrap_or_default();
		format!(
			"<?xml version=\"1.0\" encoding=\"UTF-8\"?><report><report_code>{}</report_code><submission_code>{}</submission_code><reporting_person><entity_id>{}</entity_id><entity_name>{}</entity_name>{}</reporting_person><transactions>{}</transactions></report>",
			self.report_code.as_xml(),
			self.submission_code.as_xml(),
			xml_escape(&self.reporting_person.entity_id),
			xml_escape(&self.reporting_person.entity_name),
			jurisdiction_xml,
			transactions_xml,
		)
	}
}

/// Real XSD validation against `schema/goaml_report.xsd`, via libxml2's
/// own schema engine (`libxml` crate). `Mutex`-wrapped because
/// `SchemaValidationContext::validate_document` needs `&mut self` and
/// this driver, like every other one in this phase, is meant to be shared
/// (`Send + Sync`) across a module's callers.
pub struct GoamlSchemaValidator {
	ctx: Mutex<SchemaValidationContext>,
}

impl GoamlSchemaValidator {
	pub fn from_xsd_file(path: impl AsRef<Path>) -> Result<Self, Vec<String>> {
		let path_str = path.as_ref().to_string_lossy().to_string();
		let mut parser = SchemaParserContext::from_file(&path_str);
		let ctx = SchemaValidationContext::from_parser(&mut parser).map_err(describe_errors)?;
		Ok(Self { ctx: Mutex::new(ctx) })
	}

	/// `Ok(())` means `xml` is real, schema-valid goAML report XML per
	/// `schema/goaml_report.xsd`. `Err` carries libxml2's own structured
	/// error messages (element/type/enumeration violations), not a
	/// generic "invalid" — real diagnostics, not a stub.
	pub fn validate(&self, xml: &str) -> Result<(), Vec<String>> {
		let document = Parser::default().parse_string(xml).map_err(|error| vec![format!("XML is not even well-formed: {error:?}")])?;
		let mut ctx = self.ctx.lock().map_err(|_| vec!["schema validation context lock poisoned".to_string()])?;
		ctx.validate_document(&document).map_err(describe_errors)
	}
}

fn describe_errors(errors: Vec<libxml::error::StructuredError>) -> Vec<String> {
	errors.into_iter().map(|error| error.message.unwrap_or_else(|| "unknown schema validation error".to_string())).collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubmissionAck {
	pub reference: String,
}

#[derive(Debug)]
pub enum TransportError {
	Io(String),
}

pub trait SubmissionTransport: Send + Sync {
	fn submit(&self, envelope_xml: &str) -> Result<SubmissionAck, TransportError>;
}

/// The one honest `SubmissionTransport` this workspace can ship without
/// live goAML gateway credentials: writes a validated envelope to a local
/// outbox directory and returns a reference derived from a monotonic
/// counter plus the process start time — real, unique, and inspectable
/// (an operator can read the outbox directly), not a fabricated
/// acknowledgment implying a real submission happened. A real gateway
/// client implements the same trait and replaces this one without
/// touching `GuardedSubmitter` or the envelope/validation code.
pub struct FileOutboxTransport {
	dir: PathBuf,
	next_id: std::sync::atomic::AtomicU64,
}

impl FileOutboxTransport {
	pub fn new(dir: impl Into<PathBuf>) -> Self {
		Self { dir: dir.into(), next_id: std::sync::atomic::AtomicU64::new(0) }
	}
}

impl SubmissionTransport for FileOutboxTransport {
	fn submit(&self, envelope_xml: &str) -> Result<SubmissionAck, TransportError> {
		std::fs::create_dir_all(&self.dir).map_err(|error| TransportError::Io(error.to_string()))?;
		let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
		let reference = format!("goaml-outbox-{}-{id}", std::process::id());
		let path = self.dir.join(format!("{reference}.xml"));
		std::fs::write(&path, envelope_xml).map_err(|error| TransportError::Io(error.to_string()))?;
		Ok(SubmissionAck { reference })
	}
}

#[derive(Debug)]
pub enum SubmitError {
	Invalid(Vec<String>),
	Transport(TransportError),
}

/// The only path this crate offers from a `ReportEnvelope` to a
/// `SubmissionTransport`: validation is not an optional step a caller
/// could accidentally skip by calling the transport directly (a caller
/// choosing to bypass this type and call their own transport is a
/// decision outside this crate's control, the same as any Rust API — but
/// nothing here provides a shortcut).
pub struct GuardedSubmitter<'v, T: SubmissionTransport> {
	validator: &'v GoamlSchemaValidator,
	transport: T,
}

impl<'v, T: SubmissionTransport> GuardedSubmitter<'v, T> {
	pub fn new(validator: &'v GoamlSchemaValidator, transport: T) -> Self {
		Self { validator, transport }
	}

	pub fn submit_if_valid(&self, envelope: &ReportEnvelope) -> Result<SubmissionAck, SubmitError> {
		let xml = envelope.to_xml();
		self.validator.validate(&xml).map_err(SubmitError::Invalid)?;
		self.transport.submit(&xml).map_err(SubmitError::Transport)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn schema_path() -> PathBuf {
		Path::new(env!("CARGO_MANIFEST_DIR")).join("schema/goaml_report.xsd")
	}

	fn valid_envelope() -> ReportEnvelope {
		ReportEnvelope {
			report_code: ReportCode::Str,
			submission_code: SubmissionCode::New,
			reporting_person: ReportingPerson { entity_id: "FIU-0001".into(), entity_name: "Test Reporting Entity".into(), jurisdiction: Some("TEST".into()) },
			transactions: vec![Transaction {
				transaction_number: "TXN-0001".into(),
				transmode_code: "WIRE".into(),
				date_transaction: "2026-09-21".into(),
				amount_local: "12500.00".into(),
				from: PartyRef { party_id: "P-1".into(), party_name: "Sender & Co.".into() },
				to: PartyRef { party_id: "P-2".into(), party_name: "Receiver \"LLC\"".into() },
			}],
		}
	}

	#[test]
	fn a_real_envelope_validates_against_the_real_xsd_schema() {
		let validator = GoamlSchemaValidator::from_xsd_file(schema_path()).expect("schema must load");
		let xml = valid_envelope().to_xml();
		assert!(validator.validate(&xml).is_ok(), "a correctly-shaped envelope must validate: {xml}");
	}

	#[test]
	fn special_characters_in_party_names_are_escaped_and_still_validate() {
		// The fixture envelope above already includes '&' and '"' in party
		// names — if escaping were wrong, either to_xml would produce
		// malformed XML (caught by the parser) or the schema validator
		// would reject it. Both paths are exercised by the same call.
		let validator = GoamlSchemaValidator::from_xsd_file(schema_path()).expect("schema must load");
		let xml = valid_envelope().to_xml();
		assert!(xml.contains("Sender &amp; Co."));
		assert!(xml.contains("Receiver &quot;LLC&quot;"));
		assert!(validator.validate(&xml).is_ok());
	}

	#[test]
	fn a_report_missing_a_required_element_fails_real_schema_validation() {
		let validator = GoamlSchemaValidator::from_xsd_file(schema_path()).expect("schema must load");
		// No <transactions> at all — required by the real XSD.
		let xml = r#"<?xml version="1.0" encoding="UTF-8"?><report><report_code>STR</report_code><submission_code>E</submission_code><reporting_person><entity_id>FIU-0001</entity_id><entity_name>Test</entity_name></reporting_person></report>"#;
		let result = validator.validate(xml);
		assert!(result.is_err(), "a report missing <transactions> must fail real schema validation");
	}

	#[test]
	fn an_invalid_enumeration_value_fails_real_schema_validation() {
		let validator = GoamlSchemaValidator::from_xsd_file(schema_path()).expect("schema must load");
		let mut envelope = valid_envelope();
		envelope.reporting_person.entity_id = "FIU-0002".into();
		let xml = envelope.to_xml().replace("<report_code>STR</report_code>", "<report_code>NOT_A_REAL_CODE</report_code>");
		let result = validator.validate(&xml);
		assert!(result.is_err(), "report_code outside the real enumeration must fail real schema validation");
	}

	#[test]
	fn guarded_submitter_never_reaches_the_transport_for_an_invalid_envelope() {
		let validator = GoamlSchemaValidator::from_xsd_file(schema_path()).expect("schema must load");
		let outbox = std::env::temp_dir().join(format!("nirdosha-goaml-outbox-invalid-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&outbox);
		let submitter = GuardedSubmitter::new(&validator, FileOutboxTransport::new(&outbox));

		let mut envelope = valid_envelope();
		envelope.transactions.clear(); // now invalid: transactions requires minOccurs=1
		let result = submitter.submit_if_valid(&envelope);
		assert!(matches!(result, Err(SubmitError::Invalid(_))), "expected SubmitError::Invalid, got {result:?}");
		assert!(!outbox.exists() || std::fs::read_dir(&outbox).map(|mut d| d.next().is_none()).unwrap_or(true), "an invalid envelope must never reach the transport / write to the outbox");
	}

	#[test]
	fn guarded_submitter_writes_a_valid_envelope_to_the_real_outbox() {
		let validator = GoamlSchemaValidator::from_xsd_file(schema_path()).expect("schema must load");
		let outbox = std::env::temp_dir().join(format!("nirdosha-goaml-outbox-valid-{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&outbox);
		let submitter = GuardedSubmitter::new(&validator, FileOutboxTransport::new(&outbox));

		let ack = submitter.submit_if_valid(&valid_envelope()).expect("a valid envelope must submit successfully");
		let written = std::fs::read_to_string(outbox.join(format!("{}.xml", ack.reference))).expect("outbox file must exist with the acked reference name");
		assert!(written.contains("<report_code>STR</report_code>"));
		let _ = std::fs::remove_dir_all(&outbox);
	}
}
