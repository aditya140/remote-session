import Base
import ./Model.bend as M

# SE-9: Eligible owners can approve regardless of control and cwd roots.
law owner_approval:
  for controller: Bool
  for roots: Bool
  {M.authorize(M.Approve{}, M.Facts{True{}, True{}, False{}, True{}, controller, roots, True{}, True{}, False{}, False{}, False{}}) == M.Allow{} : M.Decision}

# PO-1: A new session must use an allowed root.
law start_outside_roots:
  for controller: Bool
  {M.authorize(M.Start{}, M.Facts{True{}, True{}, False{}, True{}, controller, False{}, True{}, True{}, False{}, False{}, False{}}) == M.Forbidden{} : M.Decision}

# SE-12b: Control of an existing session does not depend on its cwd.
law existing_control:
  for roots: Bool
  {M.authorize(M.Control{}, M.Facts{True{}, True{}, False{}, True{}, False{}, roots, True{}, True{}, False{}, False{}, False{}}) == M.Allow{} : M.Decision}

# ID-6: Revoked access denies every modeled action.
law revoked_denied:
  for action: M.Action
  for owner: Bool
  for fresh: Bool
  for same_user: Bool
  for controller: Bool
  for roots: Bool
  for capability: Bool
  for host_allows: Bool
  for external: Bool
  for confirmed: Bool
  for external_confirmed: Bool
  {M.authorize(action, M.Facts{owner, fresh, True{}, same_user, controller, roots, capability, host_allows, external, confirmed, external_confirmed}) == M.Forbidden{} : M.Decision}

law non_owner_denied:
  for a: M.Action
  for fresh: Bool
  for same: Bool
  for ctrl: Bool
  for roots: Bool
  for cap: Bool
  for host: Bool
  for external: Bool
  for confirm: Bool
  for ec: Bool
  {M.authorize(a, M.Facts{False{}, fresh, False{}, same, ctrl, roots, cap, host, external, confirm, ec}) == M.Forbidden{} : M.Decision}

law stale_access_denied:
  for a: M.Action
  for ctrl: Bool
  for roots: Bool
  for cap: Bool
  for host: Bool
  for external: Bool
  for confirm: Bool
  for ec: Bool
  {M.authorize(a, M.Facts{True{}, False{}, False{}, True{}, ctrl, roots, cap, host, external, confirm, ec}) == M.AccessStale{} : M.Decision}

law unsupported_denied:
  for a: M.Action
  for ctrl: Bool
  for roots: Bool
  for external: Bool
  for confirm: Bool
  for ec: Bool
  {M.authorize(a, M.Facts{True{}, True{}, False{}, True{}, ctrl, roots, False{}, True{}, external, confirm, ec}) == M.UnsupportedCapability{} : M.Decision}

law input_requires_control:
  for roots: Bool
  {M.authorize(M.Input{}, M.Facts{True{}, True{}, False{}, True{}, False{}, roots, True{}, True{}, False{}, False{}, False{}}) == M.StaleGeneration{} : M.Decision}

law termination_needs_confirmation:
  for ctrl: Bool
  for roots: Bool
  {M.authorize(M.Terminate{}, M.Facts{True{}, True{}, False{}, True{}, ctrl, roots, True{}, True{}, False{}, False{}, False{}}) == M.Forbidden{} : M.Decision}

law external_needs_extra_confirmation:
  for ctrl: Bool
  for roots: Bool
  {M.authorize(M.Terminate{}, M.Facts{True{}, True{}, False{}, True{}, ctrl, roots, True{}, True{}, True{}, True{}, False{}}) == M.Forbidden{} : M.Decision}

law confirmed_termination_without_control:
  for roots: Bool
  {M.authorize(M.Terminate{}, M.Facts{True{}, True{}, False{}, True{}, False{}, roots, True{}, True{}, True{}, True{}, True{}}) == M.Allow{} : M.Decision}
