import Base
import ./Model.bend as M
import ../Common.bend as C

# RECORD receipt: Matching identity and epoch do not establish durable storage.
law non_durable_receipt_retained:
  {M.receipt(True{}, True{}, False{}) == M.Retained{} : M.Storage}

# RECORD-001
law retry_immutable:
  for +c: M.Chunk
  {M.upload(c) == c : M.Chunk}

# RECORD-002
law wrong_epoch_retained:
  for i: Bool
  for d: Bool
  {M.step(M.Retained{}, M.Receipt{i, False{}, d}) == M.Retained{} : M.Storage}

# RECORD-002 identity
law wrong_identity_retained:
  for e: Bool
  for d: Bool
  {M.step(M.Retained{}, M.Receipt{False{}, e, d}) == M.Retained{} : M.Storage}

# RECORD-002 positive
law valid_receipt_reclaims:
  {M.step(M.Retained{}, M.Receipt{True{}, True{}, True{}}) == M.Reclaimed{} : M.Storage}

# RECORD-003
law capacity_records_gap:
  {M.step(M.Retained{}, M.Capacity{True{}}) == M.GapCommitted{} : M.Storage}

# RECORD-003 uncertainty
law failed_metadata_is_not_durable_gap:
  {M.step(M.Retained{}, M.Capacity{False{}}) == M.UncertainLoss{} : M.Storage}

# RECORD-004
law storage_failure_keeps_execution:
  for e: M.StorageEvent
  {M.stops_execution(e) == False{} : Bool}
