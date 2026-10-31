import Base

type Action is Data:
  Observe{}
  Start{}
  Browse{}
  Control{}
  Input{}
  Resize{}
  Interrupt{}
  Approve{}
  Terminate{}

type Decision is Data:
  Allow{}
  Forbidden{}
  AccessStale{}
  UnsupportedCapability{}
  StaleGeneration{}

type Facts is Data:
  Facts{owner: Bool, access_fresh: Bool, revoked: Bool, same_os_user: Bool, controller: Bool, within_roots: Bool, capability: Bool, host_allows: Bool, external: Bool, termination_confirmed: Bool, external_termination_confirmed: Bool}

def choose(b: Bool, yes: Decision, no: Decision) -> Decision:
  match b:
    case True{}: yes
    case False{}: no

def needs_roots(a: Action) -> Bool:
  match a:
    case Start{}: True{}
    case Browse{}: True{}
    case _: False{}

def needs_control(a: Action) -> Bool:
  match a:
    case Input{}: True{}
    case Resize{}: True{}
    case Interrupt{}: True{}
    case _: False{}

def termination(a: Action, external: Bool, confirmed: Bool, external_confirmed: Bool) -> Decision:
  match a:
    case Terminate{}:
      choose(Bool.and(confirmed, Bool.or(Bool.not(external), external_confirmed)), Allow{}, Forbidden{})
    case _: Allow{}

def authorize(+a: Action, f: Facts) -> Decision:
  match f:
    case Facts{owner, fresh, revoked, same_user, controller, roots, capability, host_allows, external, confirmed, external_confirmed}:
      choose(Bool.or(revoked, Bool.or(Bool.not(owner), Bool.not(same_user))), Forbidden{},
        choose(fresh,
          choose(host_allows,
            choose(capability,
              choose(Bool.and(needs_roots(a), Bool.not(roots)), Forbidden{},
                choose(Bool.and(needs_control(a), Bool.not(controller)), StaleGeneration{},
                  termination(a, external, confirmed, external_confirmed))),
              UnsupportedCapability{}),
            Forbidden{}),
          AccessStale{}))
