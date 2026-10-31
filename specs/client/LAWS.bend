import Base
import ./Model.bend as M
import ../Common.bend as C

# CLIENT-001
law hidden_selection_cannot_act:
  for current: Nat
  for action: M.Action
  for confirmation: M.Confirmation
  {M.may_execute(False{}, current, action, confirmation) == False{} : Bool}

# CLIENT-002
law confirmation_cannot_change_action:
  for current: Nat
  for target: Nat
  {M.may_execute(True{}, current, M.KillSession{}, M.Confirmation{target, M.TakeControl{}}) == False{} : Bool}

# CLIENT-003
law matching_confirmation_allows:
  for +target: Nat
  {M.may_execute(True{}, target, M.KillSession{}, M.Confirmation{target, M.KillSession{}}) == True{} : Bool}

# CLIENT-004
law unknown_not_success:
  {M.success(M.OutcomeUnknown{}) == False{} : Bool}

# CLIENT-005
law reconnect_never_resends_input:
  for o: M.Outcome
  {M.reconnect_resends(o) == False{} : Bool}

# CLIENT-002: A confirmation for a different target never authorizes the action.
law changed_target_denied:
  for +current: Nat
  for +target: Nat
  for different: {C.eq(current, target) == False{} : Bool}
  {M.may_execute(True{}, current, M.KillSession{}, M.Confirmation{target, M.KillSession{}}) == False{} : Bool}
