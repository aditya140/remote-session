import Base

type Application is Data:
  Queued{}
  Submitted{}
  Applied{}
  Cancelled{}
  Unknown{}

type State is Data:
  Pending{}
  Closed{}
  Won{decision: Nat, application: Application}

type StepEvent is Data:
  Commit{decision: Nat}
  Deliver{permitted: Bool}
  NativeApplied{}
  ReplyLost{}
  Cancel{}

type Effect is Data:
  NoEffect{}
  CommitWinnerAuditOutbox{}
  SubmitNative{}

def application_step(a: Application, e: StepEvent) -> Application & Effect:
  match a e:
    case Queued{} Deliver{True{}}: (Submitted{}, SubmitNative{})
    case Queued{} Deliver{False{}}: (Cancelled{}, NoEffect{})
    case Queued{} Cancel{}: (Cancelled{}, NoEffect{})
    case Submitted{} NativeApplied{}: (Applied{}, NoEffect{})
    case Unknown{} NativeApplied{}: (Applied{}, NoEffect{})
    case Submitted{} ReplyLost{}: (Unknown{}, NoEffect{})
    case a0 _: (a0, NoEffect{})

def rebuild(d: Nat, pair: Application & Effect) -> State & Effect:
  (a, effect) = pair
  (Won{d, a}, effect)

def step(s: State, e: StepEvent) -> State & Effect:
  match s e:
    case Pending{} Commit{d}: (Won{d, Queued{}}, CommitWinnerAuditOutbox{})
    case Pending{} Cancel{}: (Closed{}, NoEffect{})
    case Won{d, a} e0:
      rebuild(d, application_step(a, e0))
    case s0 _: (s0, NoEffect{})

def winner(s: State) -> Maybe<Nat>:
  match s:
    case Won{d, a}: Some{d}
    case _: None{}

def advance(a: Application, e: StepEvent) -> Application:
  Pair.fst(Application, Effect, application_step(a, e))

def replay(es: List<&2, StepEvent>, a: Application) -> Application:
  match es:
    case Nil{}: a
    case Con{+e, tail}: replay(tail, advance(a, e))

def remaining(a: Application) -> Nat:
  match a:
    case Queued{}: 1n
    case _: 0n

def submissions(e: Effect) -> Nat:
  match e:
    case SubmitNative{}: 1n
    case _: 0n

def count(es: List<&2, StepEvent>, +a: Application) -> Nat:
  match es:
    case Nil{}: 0n
    case Con{+e, tail}:
      Nat.add(submissions(Pair.snd(Application, Effect, application_step(a, e))), count(tail, advance(a, e)))

def state_next(s: State, e: StepEvent) -> State:
  Pair.fst(State, Effect, step(s, e))

def state_trace(es: List<&2, StepEvent>, s: State) -> State:
  match es:
    case Nil{}: s
    case Con{e, tail}: state_trace(tail, state_next(s, e))
