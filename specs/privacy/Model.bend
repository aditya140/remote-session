import Base

type History is Data:
  History{content: Nat, held: Bool, admin_read_count: Nat}

def owner_view(s: History) -> Nat:
  match s:
    case History{content, held, count}: content

def owner_read(owns: Bool, s: History) -> Maybe<Nat>:
  match owns:
    case True{}: Some{owner_view(s)}
    case False{}: None{}

def admin_read(role: Bool, reason: Bool, audit_committed: Bool, held: Bool, case_matches: Bool) -> Bool:
  Bool.and(role, Bool.and(reason, Bool.and(audit_committed, Bool.or(Bool.not(held), case_matches))))
