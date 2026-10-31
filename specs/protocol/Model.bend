import Base

# Abstract typed tokens. This is not the production JSON or binary codec.
type Message is Data:
  Output{session: Nat, sequence: Nat}
  Input{session: Nat, sequence: Nat, host: Nat, controller: Nat}

type Token is Data:
  OutputTag{}
  InputTag{}
  Number{value: Nat}

def encode(m: Message) -> List<Token>:
  match m:
    case Output{s, n}: [OutputTag{}, Number{s}, Number{n}]
    case Input{s, n, h, c}: [InputTag{}, Number{s}, Number{n}, Number{h}, Number{c}]

def decode(ts: List<Token>) -> Maybe<Message>:
  match ts:
    case [OutputTag{}, Number{s}, Number{n}]: Some{Output{s, n}}
    case [InputTag{}, Number{s}, Number{n}, Number{h}, Number{c}]: Some{Input{s, n, h, c}}
    case _: None{}

def dispatch(valid_frame: Bool, ts: List<Token>) -> Maybe<Message>:
  match valid_frame:
    case True{}: decode(ts)
    case False{}: None{}
