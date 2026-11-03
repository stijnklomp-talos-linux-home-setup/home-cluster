# Test fixtures

`graphql-test-key.pem` is a throwaway RSA key generated for the GraphQL JWT
tests (`tests/graphql.rs`). It signs test tokens only: it is not a deployment
secret, and the service binary never reads it.
