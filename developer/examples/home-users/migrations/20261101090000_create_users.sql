-- The `users` reference subgraph's entity table (ticket #14).
--
-- `id` is the federation key; the GraphQL `User` entity is `@key(fields: "id")`.
-- Email is unique so the seed data and future identity linking can address a
-- user by address.
CREATE TABLE "users" (
    "id" BIGSERIAL PRIMARY KEY,
    "email" TEXT NOT NULL UNIQUE,
    "name" TEXT NOT NULL,
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);
