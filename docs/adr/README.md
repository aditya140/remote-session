# Implementation decisions

These records define the implementation baseline for [task 002](../../tasks/002-resolve-runtime-and-protocol-contracts.md). A selected design is not evidence that the behavior exists. External validation and release scheduling remain separate gates.

| Record | Subject | Status |
| --- | --- | --- |
| [0001](0001-cli-integration.md) | Existing CLI integration and toolchain | Corporate inspection waived for the standalone path by 0007. |
| [0002](0002-state-authority.md) | Authority, starts, control, approvals, and access freshness | Implemented and exercised in Linux component/composed fixtures. |
| [0003](0003-session-lifetime-and-ipc.md) | Independent process lifetime and private local IPC | Linux process/IPC proofs pass; actual platform service qualification is external. |
| [0004](0004-wire-contract.md) | Public protocol, numeric representation, limits, and compatibility | Implemented with conformance vectors and generated schemas. |
| [0005](0005-recording-and-retention.md) | Recording, receipts, protected reads, holds, and release gate | Linux synchronous recording/holds proofs pass; production durability/scheduling remain external. |
| [0006](0006-deployment-and-readiness.md) | Development topology, secrets, readiness, and migrations | Four-gateway development deployment, rolling/outage/restore proofs pass. |
| [0007](0007-standalone-preview.md) | Standalone TUI and new composition root | User-selected direction; removes the external CLI prerequisite. |
| [0008](0008-development-compose-networks.md) | Private development namespaces and generated secrets | Implemented and exercised in the four-gateway deployment. |
| [0009](0009-revocation-completion.md) | Conservative propagation completion from original authority expiry | Linux implementation; real-process qualification in task 019. |
| [0010](0010-browser-console.md) | Gateway-hosted admin and owner browser companion | User-approved scope; implementation and evidence in task 040. |

Use the [operation matrix](../contracts/operation-matrix.md) to implement authorization. Use the [failure walkthroughs](../contracts/failure-walkthroughs.md) to build conformance and fault tests.
