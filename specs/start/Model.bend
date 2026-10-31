import Base
import ../Common.bend as C

# One record per trusted (owner, machine, idempotency key).
type Phase is Data:
  Accepted{}
  Launching{}
  Running{}
  Failed{}
  Unknown{}
  SessionLost{}

type Record is Data:
  Record{fingerprint: Nat, session: Nat, phase: Phase}

type StepEvent is Data:
  Dispatch{}
  NativeRunning{}
  NativeFailed{}
  NativeLost{}
  ReplyLost{}
  Crash{}
  Lookup{}

type Effect is Data:
  NoLaunch{}
  Launch{}

def phase_step(p: Phase, e: StepEvent) -> Phase & Effect:
  match p e:
    case Accepted{} Dispatch{}: (Launching{}, Launch{})
    case Launching{} NativeRunning{}: (Running{}, NoLaunch{})
    case Launching{} NativeFailed{}: (Failed{}, NoLaunch{})
    case Unknown{} NativeRunning{}: (Running{}, NoLaunch{})
    case Unknown{} NativeFailed{}: (Failed{}, NoLaunch{})
    case Running{} NativeLost{}: (SessionLost{}, NoLaunch{})
    case Running{} Crash{}: (Unknown{}, NoLaunch{})
    case Launching{} Crash{}: (Unknown{}, NoLaunch{})
    case p0 _: (p0, NoLaunch{})

def rebuild(f: Nat, s: Nat, pair: Phase & Effect) -> Record & Effect:
  (p, effect) = pair
  (Record{f, s, p}, effect)

def step(r: Record, e: StepEvent) -> Record & Effect:
  match r:
    case Record{f, s, p}:
      rebuild(f, s, phase_step(p, e))

def identity(r: Record) -> Nat & Nat:
  match r:
    case Record{f, s, p}: (f, s)

def next(r: Record, e: StepEvent) -> Record:
  Pair.fst(Record, Effect, step(r, e))

def replay(es: List<&2, StepEvent>, r: Record) -> Record:
  match es:
    case Nil{}: r
    case Con{+e, tail}: replay(tail, next(r, e))

def same_request(r: Record, requested: Nat) -> Bool:
  match r:
    case Record{f, s, p}: C.eq(f, requested)

def remaining(p: Phase) -> Nat:
  match p:
    case Accepted{}: 1n
    case _: 0n

def launched(e: Effect) -> Nat:
  match e:
    case Launch{}: 1n
    case NoLaunch{}: 0n

def launches(es: List<&2, StepEvent>, +p: Phase) -> Nat:
  match es:
    case Nil{}: 0n
    case Con{+e, tail}:
      Nat.add(launched(Pair.snd(Phase, Effect, phase_step(p, e))), launches(tail, Pair.fst(Phase, Effect, phase_step(p, e))))
