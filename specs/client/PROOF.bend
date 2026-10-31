import Base
import ./Model.bend as M
import ../Common.bend as C
import ./LAWS.bend as Laws

def Laws.hidden_selection_cannot_act(current, action, confirmation):
  match confirmation:
    case M.Confirmation{t, a}: {==}

def Laws.confirmation_cannot_change_action(current, target): {==}

def Laws.matching_confirmation_allows(target): C.eq_self(target)

def Laws.unknown_not_success(): {==}

def Laws.reconnect_never_resends_input(o): {==}

def Laws.changed_target_denied(current, target, different): different
