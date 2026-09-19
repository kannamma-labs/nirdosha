//! Admission control with an explicit non-sheddable safety floor.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkClass { AsyncScoringDepth, NotifyObligations, Audit, L1Enforcement }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Admission { Admit, Shed }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionController { pub max_in_flight: u64, pub in_flight: u64 }

impl AdmissionController {
    pub fn new(max_in_flight: u64) -> Self { Self { max_in_flight, in_flight: 0 } }

    pub fn acquire(&mut self, class: WorkClass) -> Admission {
        if matches!(class, WorkClass::Audit | WorkClass::L1Enforcement) { self.in_flight += 1; return Admission::Admit; }
        if self.in_flight >= self.max_in_flight { Admission::Shed } else { self.in_flight += 1; Admission::Admit }
    }

    pub fn release(&mut self) { self.in_flight = self.in_flight.saturating_sub(1); }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheds_optional_work_but_never_audit_or_l1() {
        let mut controller = AdmissionController::new(1);
        assert_eq!(controller.acquire(WorkClass::AsyncScoringDepth), Admission::Admit);
        assert_eq!(controller.acquire(WorkClass::NotifyObligations), Admission::Shed);
        assert_eq!(controller.acquire(WorkClass::Audit), Admission::Admit);
        assert_eq!(controller.acquire(WorkClass::L1Enforcement), Admission::Admit);
        controller.release();
        assert_eq!(controller.in_flight, 2);
    }
}