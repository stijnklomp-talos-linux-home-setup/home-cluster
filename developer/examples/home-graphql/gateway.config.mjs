import { readFile } from "node:fs/promises";

import {
  createRemoteJwksSigningKeyProvider,
  defineConfig,
} from "@graphql-hive/gateway";
import { GraphQLError } from "graphql";

// The supergraph ConfigMap is mounted as a directory (never subPath), so the
// kubelet's atomic symlink swap is visible through a fresh open() on every
// poll. A plain path string would make the CLI use fs.watch and delete the
// polling interval; a function source keeps polling and survives the swap.
const supergraphPath =
  process.env.SUPERGRAPH_PATH ?? "/gateway/supergraph/supergraph.graphql";

// The issuer is deployment input: there is no committed default, because a
// real Cognito pool id in this public repo would be both an environment leak
// and a way to silently validate tokens against the wrong pool. The chart
// values carry `<region>`/`<pool>` placeholders; a deployment that forgets to
// replace them fails at startup instead of running with a broken JWKS.
const issuer = process.env.JWT_ISSUER?.trim();
if (!issuer) {
  throw new Error(
    "JWT_ISSUER is required (Cognito issuer or the verification test issuer)",
  );
}
if (issuer.includes("<") || issuer.includes(">")) {
  throw new Error(
    `JWT_ISSUER still contains a placeholder (${issuer}); set the real Cognito issuer`,
  );
}
const jwksUri =
  process.env.JWKS_URI?.trim() || `${issuer}/.well-known/jwks.json`;
const audience = process.env.JWT_AUDIENCE?.trim() || undefined;
const clientId = process.env.JWT_CLIENT_ID?.trim() || undefined;

const unauthorized = () =>
  new GraphQLError("Invalid authentication token provided", {
    extensions: {
      code: "UNAUTHENTICATED",
      http: { status: 401 },
    },
  });

// The Yoga JWT plugin returns 400 for a token that cannot even be decoded.
// Normalize every unusable Authorization header to 401: a non-JWT string, a
// malformed segment, or an empty Bearer value is an authentication failure,
// not a syntactically bad request.
function extractBearerToken({ request }) {
  const header = request?.headers.get("authorization");
  if (!header) {
    return undefined;
  }
  const [scheme, token] = header.split(" ").map((part) => part.trim());
  if (!token || scheme?.toLowerCase() !== "bearer") {
    throw unauthorized();
  }
  const segments = token.split(".");
  if (segments.length !== 3 || segments.some((segment) => !segment)) {
    throw unauthorized();
  }
  try {
    JSON.parse(Buffer.from(segments[0], "base64url").toString());
    JSON.parse(Buffer.from(segments[1], "base64url").toString());
  } catch {
    throw unauthorized();
  }
  // Hive/Yoga token lookups return `{ prefix, token }` (see
  // `extractFromHeader`); a raw string makes every token look absent.
  return { prefix: "Bearer", token };
}

const tokenVerification = {
  issuer,
  algorithms: ["RS256"],
};
if (audience) {
  tokenVerification.audience = audience;
}

// The JWT plugin validates the signature, issuer and expiry in onRequestParse,
// before query planning. `reject` stays on, so a missing or invalid token is a
// 401 and no subgraph request is made. genericAuth then enforces the
// @requiresScopes directives for valid tokens.
// Client-id/audience enforcement runs in `onExecute`, after the JWT plugin has
// populated `context.jwt`. An `onContextBuilding` hook can run before that and
// would reject every valid token. Cognito access tokens have no `aud`; their
// `client_id` is the audience.
const enforceClientId = {
  onExecute({ args }) {
    if (!clientId) {
      return;
    }
    const payload = args.contextValue?.jwt?.payload;
    if (!payload || payload.client_id !== clientId) {
      throw unauthorized();
    }
  },
};

export const gatewayConfig = defineConfig({
  supergraph: () => readFile(supergraphPath, "utf8"),
  // 10_000 is the smallest value the 2.15.4 CLI accepts.
  pollingInterval: 10_000,
  host: "0.0.0.0",
  port: 4000,

  jwt: {
    tokenLookupLocations: [extractBearerToken],
    signingKeyProviders: [createRemoteJwksSigningKeyProvider({ jwksUri })],
    tokenVerification,
    // A missing/invalid token is a 401 before any query planning or subgraph
    // call. Granular scope checks are layered on top by genericAuth.
    reject: { missingToken: true, invalidToken: true },
    // The raw header is forwarded below; do not forward the parsed payload.
    forward: { payload: false, token: false },
  },

  // Federation @requiresScopes enforcement. The default scope claim is the
  // space-delimited `scope` string Cognito access tokens carry; `scp` arrays
  // are accepted too. `rejectUnauthenticated` keeps a denied field a hard 401
  // instead of a partial 200 response.
  genericAuth: {
    mode: "protect-granular",
    resolveUserFn: (ctx) => {
      const payload = ctx.jwt?.payload;
      if (!payload) {
        return undefined;
      }
      // Belt and braces: a token minted for another app client is not a user
      // of this gateway even if its signature and issuer are valid.
      if (clientId && payload.client_id !== clientId) {
        return undefined;
      }
      return payload;
    },
    extractScopes: (payload) => {
      const scopes = payload?.scope ?? payload?.scp;
      if (typeof scopes === "string") {
        return scopes.split(/\s+/).filter(Boolean);
      }
      return Array.isArray(scopes) ? scopes : [];
    },
    rejectUnauthenticated: true,
  },

  // Every subgraph validates the JWT again. Only the raw, already-validated
  // Authorization header crosses the boundary; no trusted identity headers.
  propagateHeaders: {
    fromClientToSubgraphs({ request }) {
      const authorization = request.headers.get("authorization");
      return authorization ? { authorization } : undefined;
    },
  },

  plugins: () => [enforceClientId],

  healthCheckEndpoint: "/healthcheck",
  readinessCheckEndpoint: "/readiness",
});
