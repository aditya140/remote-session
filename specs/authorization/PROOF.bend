import Base
import ./Model.bend as M
import ./LAWS.bend as Laws

def Laws.owner_approval(controller, roots):
  {==}

def Laws.start_outside_roots(controller):
  {==}

def Laws.existing_control(roots):
  {==}

def Laws.revoked_denied(action, owner, fresh, same_user, controller, roots, capability, host_allows, external, confirmed, external_confirmed):
  {==}

def Laws.non_owner_denied(a, fresh, same, ctrl, roots, cap, host, external, confirm, ec):
  {==}

def Laws.stale_access_denied(a, ctrl, roots, cap, host, external, confirm, ec):
  {==}

def Laws.unsupported_denied(a, ctrl, roots, external, confirm, ec):
  {==}

def Laws.input_requires_control(roots):
  {==}

def Laws.termination_needs_confirmation(ctrl, roots):
  {==}

def Laws.external_needs_extra_confirmation(ctrl, roots):
  {==}

def Laws.confirmed_termination_without_control(roots):
  {==}
