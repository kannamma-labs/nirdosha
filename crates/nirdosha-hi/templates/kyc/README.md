# KYC / Identity Verification Platform

A certified Nirdosha project template for customer onboarding, identity
verification, risk scoring and ongoing due diligence.

## Scope

This template covers the full KYC case lifecycle:

1. **Application Management** – create, submit and track KYC cases.
2. **Identity & Data Capture** – personal/corporate details, addresses,
   FATCA/CRS declarations and PEP self-declaration.
3. **Document Vault** – POI/POA upload, expiry tracking and verification
   status.
4. **Risk Rating Engine** – score-based Low / Medium / High classification.
5. **Review & Approval Workflow** – maker-checker queue with RFI support.
6. **Re-KYC & Expiry Management** – periodic reviews and event-driven refresh.
7. **Audit & Reporting** – immutable state-transition trail.

## Modules

- `core` (required) – all build-first capabilities above.
- `document_verification` (optional, **Buy**) – Onfido, Jumio, Trulioo,
  HyperVerge, IDfy, Sumsub.
- `sanctions_pep` (optional, **Buy**) – Refinitiv World-Check,
  ComplyAdvantage, Dow Jones.
- `government_id` (optional, **Integrate**) – Aadhaar, DigiLocker, eID.
- `screening_match` (optional, **Buy**) – fuzzy matching algorithm.

## Jurisdictions

Pre-configured country parameters for:

- `IN` – India (PMLA + Aadhaar)
- `US` – United States (BSA/CIP)
- `GB` – United Kingdom (MLR)
- `SG` – Singapore (MAS)

Select the jurisdiction in the composer; it drives the default country,
logging contract and certified primitive behaviour.
