import Base
import ./Model.bend as M
import ../Common.bend as C

# START-001
law identity_step:
  for +r: M.Record
  for e: M.StepEvent
  {M.identity(M.next(r, e)) == M.identity(r) : Nat & Nat}

# START-001 trace
law identity_trace:
  for es: List<&2, M.StepEvent>
  for +r: M.Record
  {M.identity(M.replay(es, r)) == M.identity(r) : Nat & Nat}

# START-002
law unknown_no_relaunch:
  {M.phase_step(M.Unknown{}, M.Dispatch{}) == (M.Unknown{}, M.NoLaunch{}) : M.Phase & M.Effect}

# START-003
law accepted_launch:
  {M.phase_step(M.Accepted{}, M.Dispatch{}) == (M.Launching{}, M.Launch{}) : M.Phase & M.Effect}

# START-003 failure
law ambiguous_crash:
  {M.phase_step(M.Launching{}, M.Crash{}) == (M.Unknown{}, M.NoLaunch{}) : M.Phase & M.Effect}

# START-004
law lookup_read_only:
  for +p: M.Phase
  {M.phase_step(p, M.Lookup{}) == (p, M.NoLaunch{}) : M.Phase & M.Effect}

# START-002: At most one launch effect over any finite trace of one record.
law launch_budget:
  for es: List<&2, M.StepEvent>
  for +p: M.Phase
  {C.le(M.launches(es, p), M.remaining(p)) == True{} : Bool}

law matching_fingerprint:
  for +f: Nat
  for s: Nat
  for p: M.Phase
  {M.same_request(M.Record{f, s, p}, f) == True{} : Bool}

# START-003: A previous running observation is insufficient after owner loss.
law running_crash_requires_evidence:
  {M.phase_step(M.Running{}, M.Crash{}) == (M.Unknown{}, M.NoLaunch{}) : M.Phase & M.Effect}
