import Base
import ./Model.bend as M
import ../Common.bend as C

# HISTORY-001
law hold_a_prevents_purge:
  for +b: Bool
  for +age: Bool
  {M.step(M.Item{True{}, True{}, b, age}, M.Purge{}) == M.Item{True{}, True{}, b, age} : M.Item}

# HISTORY-002
law remaining_hold_survives_release:
  for +age: Bool
  {M.step(M.step(M.Item{True{}, True{}, True{}, age}, M.ReleaseA{}), M.Purge{}) == M.Item{True{}, False{}, True{}, age} : M.Item}

# HISTORY-003
law eligible_purge:
  {M.step(M.Item{True{}, False{}, False{}, True{}}, M.Purge{}) == M.Item{False{}, False{}, False{}, True{}} : M.Item}

# HISTORY-004
law hold_cannot_restore_purged_content:
  {M.step(M.step(M.Item{True{}, False{}, False{}, True{}}, M.Purge{}), M.PlaceA{}) == M.Item{False{}, True{}, False{}, True{}} : M.Item}

# HISTORY-005
law upload_does_not_reset_age:
  for +s: M.Item
  {M.step(s, M.UploadRetry{}) == s : M.Item}
