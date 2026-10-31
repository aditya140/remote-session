import Base
import ./Model.bend as M
import ../Common.bend as C

# LIFE-001
law network_no_kill:
  for e: M.NetworkEvent
  {M.is_kill(M.effect(M.network(e))) == False{} : Bool}

# LIFE-001 trace
law network_trace_no_kill:
  for es: List<&2, M.NetworkEvent>
  {M.kills(es) == False{} : Bool}

# LIFE-002
law explicit_termination:
  {M.effect(M.Terminate{True{}}) == M.Kill{} : M.Effect}

# LIFE-002 confirmation
law no_confirmation_no_kill:
  {M.effect(M.Terminate{False{}}) == M.Keep{} : M.Effect}

# LIFE-003
law owner_loss_unknown:
  {M.effect(M.OwnerCrash{}) == M.ReconcileUnknown{} : M.Effect}
