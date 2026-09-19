pub mod conformance;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity { Error, Warning }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding { pub pass: String, pub severity: Severity, pub message: String, pub item: Option<String> }

impl Finding { fn error(pass: &str, message: impl Into<String>, item: Option<String>) -> Self { Self { pass: pass.into(), severity: Severity::Error, message: message.into(), item } } }

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PolicyView { pub id: String, pub action: String, pub resource: String, pub purpose: Option<String>, pub effect: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PortView { pub name: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DriverView { pub port: String, pub vendor: String, pub version: String, pub capabilities: Vec<String>, pub lineage_support: String }
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RegistryView { pub policies: Vec<PolicyView>, pub ports: Vec<PortView>, pub drivers: Vec<DriverView> }

impl RegistryView { pub fn from_json(value: &str) -> Result<Self, serde_json::Error> { serde_json::from_str(value) } }

pub trait VerifyPass { fn name(&self) -> &'static str; fn run(&self, registry: &RegistryView) -> Vec<Finding>; }

struct V1; struct V2; struct V3; struct V4; struct V5; struct V6; struct V7; struct V8;

impl VerifyPass for V1 { fn name(&self) -> &'static str { "V1" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.policies.iter().filter(|p| p.id.is_empty() || p.action.is_empty() || p.resource.is_empty()).map(|p| Finding::error(self.name(), "policy record is missing an identity, action, or resource", Some(p.id.clone()))).collect() } }
impl VerifyPass for V2 { fn name(&self) -> &'static str { "V2" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.policies.iter().filter(|p| p.effect != "allow" && p.effect != "deny").map(|p| Finding::error(self.name(), "policy effect is outside the closed set", Some(p.id.clone()))).collect() } }
impl VerifyPass for V3 { fn name(&self) -> &'static str { "V3" } fn run(&self, _registry: &RegistryView) -> Vec<Finding> { Vec::new() } }
impl VerifyPass for V4 { fn name(&self) -> &'static str { "V4" } fn run(&self, _registry: &RegistryView) -> Vec<Finding> { Vec::new() } }
impl VerifyPass for V5 { fn name(&self) -> &'static str { "V5" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.drivers.iter().filter(|d| d.lineage_support.is_empty()).map(|d| Finding::error(self.name(), "driver manifest omits lineage support", Some(d.vendor.clone()))).collect() } }
impl VerifyPass for V6 { fn name(&self) -> &'static str { "V6" } fn run(&self, _registry: &RegistryView) -> Vec<Finding> { Vec::new() } }
impl VerifyPass for V7 { fn name(&self) -> &'static str { "V7" } fn run(&self, _registry: &RegistryView) -> Vec<Finding> { Vec::new() } }
impl VerifyPass for V8 { fn name(&self) -> &'static str { "V8" } fn run(&self, registry: &RegistryView) -> Vec<Finding> { registry.ports.iter().filter_map(|port| { let count = registry.drivers.iter().filter(|driver| driver.port == port.name).count(); (count < 2).then(|| Finding::error(self.name(), format!("port requires two driver manifests, found {count}"), Some(port.name.clone()))) }).collect() } }

pub fn standard_passes() -> Vec<Box<dyn VerifyPass>> { vec![Box::new(V1), Box::new(V2), Box::new(V3), Box::new(V4), Box::new(V5), Box::new(V6), Box::new(V7), Box::new(V8)] }
pub fn verify(registry: &RegistryView) -> Vec<Finding> { standard_passes().into_iter().flat_map(|pass| pass.run(registry)).collect() }

#[cfg(test)]
mod tests {
	use super::*;
	#[test]
	fn v8_requires_two_drivers_per_port() { let registry = RegistryView { ports: vec![PortView { name: "store".into() }], drivers: vec![DriverView { port: "store".into(), vendor: "memory".into(), version: "1".into(), lineage_support: "datasets".into(), ..Default::default() }], ..Default::default() }; assert_eq!(verify(&registry).iter().filter(|f| f.pass == "V8").count(), 1); }
	#[test]
	fn v5_requires_lineage_support() { let registry = RegistryView { drivers: vec![DriverView { port: "store".into(), vendor: "memory".into(), version: "1".into(), ..Default::default() }], ..Default::default() }; assert!(verify(&registry).iter().any(|f| f.pass == "V5")); }
	#[test]
	fn valid_catalog_has_no_findings() { let registry = RegistryView { policies: vec![PolicyView { id: "read".into(), action: "read".into(), resource: "orders".into(), effect: "allow".into(), purpose: None }], ports: vec![PortView { name: "store".into() }], drivers: vec![DriverView { port: "store".into(), vendor: "memory".into(), version: "1".into(), lineage_support: "datasets".into(), ..Default::default() }, DriverView { port: "store".into(), vendor: "remote".into(), version: "1".into(), lineage_support: "datasets".into(), ..Default::default() }] }; assert!(verify(&registry).is_empty()); }
}
