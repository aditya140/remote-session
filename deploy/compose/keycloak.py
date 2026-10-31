"""Local identity configuration. This does not change production Entra support."""
import json
import secrets
import time
import urllib.parse

TENANT = "11111111-1111-4111-8111-111111111111"
DIRECTORY_CLIENT = "33333333-3333-4333-8333-333333333333"
CLIENT = "qgr-local"
AUDIENCE = "api://qgr-local"
SCOPE = AUDIENCE + "/Remote.Control"
REALM = "/realms/qgr-local"
USERS = {"alice": "22222222-2222-4222-8222-222222222222", "bob": "44444444-4444-4444-8444-444444444444"}


def mapper(name, kind, config):
    return {"name": name, "protocol": "openid-connect", "protocolMapper": kind,
            "config": {"access.token.claim": "true", "id.token.claim": "false",
                       "userinfo.token.claim": "false", **config}}


def configure(args, put, private, directory_secret):
    origin = f"https://localhost:{args.identity_port}"
    issuer = origin + REALM
    credentials = {name: secrets.token_urlsafe(24) for name in (*USERS, "admin")}
    database_password = secrets.token_hex(32)
    scope = {"name": SCOPE, "protocol": "openid-connect",
             "attributes": {"include.in.token.scope": "true", "display.on.consent.screen": "true"},
             "protocolMappers": [
                 mapper("remote audience", "oidc-audience-mapper", {"included.custom.audience": AUDIENCE}),
                 mapper("tenant", "oidc-hardcoded-claim-mapper", {"claim.name": "tid", "claim.value": TENANT, "jsonType.label": "String"}),
                 mapper("remote permission", "oidc-hardcoded-claim-mapper", {"claim.name": "scp", "claim.value": "Remote.Control", "jsonType.label": "String"}),
                 # Keycloak omits nbf by default. The local realm cannot issue a
                 # token valid before its creation; iat/exp remain per-token.
                 mapper("realm not before", "oidc-hardcoded-claim-mapper", {"claim.name": "nbf", "claim.value": str(int(time.time())), "jsonType.label": "long"}),
                 mapper("stable user", "oidc-usermodel-property-mapper", {"user.attribute": "id", "claim.name": "oid", "jsonType.label": "String"}),
                 mapper("subject", "oidc-usermodel-property-mapper", {"user.attribute": "id", "claim.name": "sub", "jsonType.label": "String"}),
             ]}
    realm = {"realm": "qgr-local", "enabled": True, "sslRequired": "external",
             "registrationAllowed": False, "resetPasswordAllowed": False,
             "bruteForceProtected": True, "accessTokenLifespan": 900,
             "clientScopes": [scope, {"name": "https://edge:9443/.default", "protocol": "openid-connect"}],
             "clients": [
                 {"clientId": CLIENT, "protocol": "openid-connect", "enabled": True,
                  "publicClient": True, "standardFlowEnabled": True,
                  "directAccessGrantsEnabled": False, "serviceAccountsEnabled": False,
                  # RFC 8252: Keycloak permits an ephemeral port for a loopback
                  # IP redirect registered without a port. A wildcard port fails.
                  "redirectUris": ["http://127.0.0.1/oauth/callback"],
                  "attributes": {"pkce.code.challenge.method": "S256", "oauth2.device.authorization.grant.enabled": "true"},
                  "defaultClientScopes": [], "optionalClientScopes": [SCOPE, "offline_access"]},
                 {"clientId": DIRECTORY_CLIENT, "protocol": "openid-connect", "enabled": True,
                  "publicClient": False, "secret": directory_secret, "standardFlowEnabled": False,
                  "directAccessGrantsEnabled": False, "serviceAccountsEnabled": True,
                  "defaultClientScopes": ["roles"], "optionalClientScopes": ["https://edge:9443/.default"]},
             ],
             "users": [{"id": oid, "username": name, "enabled": True, "emailVerified": True,
                        "realmRoles": ["offline_access"],
                        "firstName": name.capitalize(), "lastName": "Local", "email": name + "@localhost.invalid",
                        "credentials": [{"type": "password", "value": credentials[name], "temporary": False}]}
                       for name, oid in USERS.items()] + [
                 {"username": "service-account-" + DIRECTORY_CLIENT, "enabled": True,
                  "serviceAccountClientId": DIRECTORY_CLIENT,
                  "clientRoles": {"realm-management": ["view-users", "query-users"]}}]}
    put("identity", "qgr-local-realm.json", realm)
    put("identity", "keycloak.conf", f"db=postgres\ndb-url=jdbc:postgresql://postgres:5432/qgr_keycloak\ndb-username=qgr_keycloak\ndb-password={database_password}\nhttp-enabled=true\nhttp-port=8080\nhostname={origin}\nproxy-headers=xforwarded\nbootstrap-admin-username=admin\nbootstrap-admin-password={credentials['admin']}\n")
    put("postgres", "10-keycloak.sql", f"CREATE ROLE qgr_keycloak LOGIN PASSWORD '{database_password}';\nCREATE DATABASE qgr_keycloak OWNER qgr_keycloak;\n")
    put("gateway", "identity.json", {"issuer": issuer, "tenant": TENANT, "audience": AUDIENCE,
        "remote_scope": "Remote.Control", "admin_role": "Remote.Admin",
        "jwks_url": "https://edge:9443" + REALM + "/protocol/openid-connect/certs", "ca_bundle": "/run/qgr/ca.pem"})
    put("gateway", "directory.json", {"tenant": TENANT, "client_id": DIRECTORY_CLIENT,
        "credential_file": "/run/qgr/graph-secret", "token_url": "https://edge:9443" + REALM + "/protocol/openid-connect/token",
        "graph_base": "https://edge:9443/v1.0", "ca_bundle": "/run/qgr/ca.pem"})
    private(args.state / "users.json", json.dumps(credentials, indent=2) + "\n")
    private(args.state / "oauth.json", json.dumps({"client_id": CLIENT, "remote_scope": SCOPE,
        "authorization_endpoint": issuer + "/protocol/openid-connect/auth",
        "token_endpoint": issuer + "/protocol/openid-connect/token",
        "device_endpoint": issuer + "/protocol/openid-connect/auth/device",
        "device_pkce": True,
        "ca_bundle": str(args.state.resolve() / "certificates/ca.pem")}, indent=2) + "\n")


def ready(stack):
    discovery = stack.http(REALM + "/.well-known/openid-configuration", identity=True)
    if discovery.get("issuer") != f"https://localhost:{stack.args.identity_port}" + REALM:
        return False
    token = stack.http(REALM + "/protocol/openid-connect/token", identity=True, form={
        "grant_type": "client_credentials", "client_id": DIRECTORY_CLIENT,
        "client_secret": (stack.args.state / "input/gateway/graph-secret").read_text(),
        "scope": "https://edge:9443/.default"})["access_token"]
    query = urllib.parse.urlencode({"$select": "id,accountEnabled"})
    user = stack.http("/v1.0/users/" + USERS["alice"] + "?" + query, identity=True, token=token)
    return user == {"id": USERS["alice"], "accountEnabled": True}
