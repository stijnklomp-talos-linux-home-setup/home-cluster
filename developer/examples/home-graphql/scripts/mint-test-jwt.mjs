#!/usr/bin/env node
// Mint a test JWT for the live verification. RS256, kid `test-key`, signed
// with the committed test fixture (the same key home-users tests use).
//
//   node scripts/mint-test-jwt.mjs \
//     [--key path] [--iss iss] [--aud aud[,aud...]] [--sub sub] \
//     [--email email] [--scope "users:read orders:read"] [--client-id id] \
//     [--ttl seconds]
//
// The token is printed to stdout. It is a test token: the key is public and
// the in-cluster test JWKS serves its public half. The defaults match the
// deployed verification overlay: Cognito access-token shape (no `aud`,
// `client_id: home-test-client`) and the scopes for both subgraphs. Pass
// `--aud home-users,home-orders` for the older audience-based overlay.

import { readFileSync } from "node:fs";
import { createSign } from "node:crypto";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function arg(name, fallback) {
  const index = process.argv.indexOf(`--${name}`);
  return index === -1 ? fallback : process.argv[index + 1];
}

const keyPath = resolve(root, arg("key", "tests/fixtures/graphql-test-key.pem"));
const issuer = arg("iss", "https://test-issuer.home.test");
// The verification overlay leaves `jwtAudience` empty, so the default token
// has no `aud` and is validated by `client_id` instead. A comma-separated
// `--aud` becomes a JWT `aud` array for the older audience-based overlay.
const audiences = arg("aud", "")
  .split(",")
  .map((value) => value.trim())
  .filter(Boolean);
const audience = audiences.length === 1 ? audiences[0] : audiences;
const subject = arg("sub", "1");
const email = arg("email", "ada@example.test");
const scope = arg("scope", "users:read orders:read");
const clientId = arg("client-id", "home-test-client");
const ttl = Number(arg("ttl", "3600"));

const now = Math.floor(Date.now() / 1000);
const encode = (value) =>
  Buffer.from(JSON.stringify(value)).toString("base64url");

const header = encode({ alg: "RS256", typ: "JWT", kid: "test-key" });
const claims = {
  iss: issuer,
  sub: subject,
  email,
  email_verified: true,
  scope,
  iat: now,
  exp: now + ttl,
};
if (audiences.length > 0) {
  claims.aud = audience;
}
if (clientId) {
  claims.client_id = clientId;
}
const payload = encode(claims);

const signature = createSign("RSA-SHA256")
  .update(`${header}.${payload}`)
  .sign(readFileSync(keyPath, "utf8"))
  .toString("base64url");

process.stdout.write(`${header}.${payload}.${signature}\n`);
