import Base

def enabled(client: Bool, gateway: Bool, host: Bool, adapter: Bool) -> Bool:
  Bool.and(client, Bool.and(gateway, Bool.and(host, adapter)))

def negotiate(required: Bool, available: Bool) -> Bool:
  Bool.or(Bool.not(required), available)
