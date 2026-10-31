"""Security boundaries of the optional local identity adapter and realm."""
import argparse
import json
import os
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "deploy/compose"))
import directory
import keycloak
from stack import initialize


class LocalIdentity(unittest.TestCase):
    def setUp(self):
        self.previous_umask = os.umask(0o077)

    def tearDown(self):
        os.umask(self.previous_umask)

    def test_directory_accepts_encoded_query_and_rejects_ambiguous_selection(self):
        oid = keycloak.USERS["alice"]
        path = "/v1.0/users/" + oid
        for query in ("$select=id,accountEnabled", "%24select=id%2CaccountEnabled"):
            self.assertEqual(directory.object_id(path + "?" + query), oid)
        for query in ("", "$select=id", "$select=id,accountEnabled&extra=1",
                      "$select=id,accountEnabled&$select=id,accountEnabled"):
            self.assertIsNone(directory.object_id(path + "?" + query))

    def test_directory_requires_exact_boolean_and_matching_identity(self):
        oid = keycloak.USERS["alice"]
        for value in (True, False):
            self.assertEqual(directory.account(json.dumps({"id": oid, "enabled": value}), oid),
                             {"id": oid, "accountEnabled": value})
        for body in ({"id": oid}, {"id": oid, "enabled": "false"}, {"id": oid, "enabled": 1},
                     {"id": keycloak.USERS["bob"], "enabled": True}):
            with self.assertRaises(ValueError):
                directory.account(json.dumps(body), oid)

    def test_private_persistent_initialization_and_provider_boundary(self):
        with tempfile.TemporaryDirectory() as root:
            args = argparse.Namespace(state=Path(root) / "state", project="qgr-test",
                                      edge_port=8443, identity_port=9443, identity="keycloak")
            state = initialize(args)
            credentials = (args.state / "users.json").read_bytes()
            self.assertEqual(initialize(args), state)
            self.assertEqual((args.state / "users.json").read_bytes(), credentials)
            for path in args.state.rglob("*"):
                self.assertEqual(path.stat().st_mode & 0o077, 0)
            realm = json.loads((args.state / "input/identity/qgr-local-realm.json").read_text())
            client = next(c for c in realm["clients"] if c["clientId"] == keycloak.CLIENT)
            self.assertFalse(client["directAccessGrantsEnabled"])
            self.assertFalse(client["serviceAccountsEnabled"])
            self.assertEqual(client["attributes"]["pkce.code.challenge.method"], "S256")
            for mapper in realm["clientScopes"][0]["protocolMappers"]:
                self.assertEqual(mapper["config"]["id.token.claim"], "false")
            args.identity = "fixture"
            with self.assertRaisesRegex(RuntimeError, "implicit reconfiguration"):
                initialize(args)


if __name__ == "__main__":
    unittest.main()
