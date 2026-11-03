-- The `orders` subgraph's entity table (ticket #16).
--
-- `id` is the federation key; the GraphQL `Order` entity is `@key(fields: "id")`.
-- `userId` is a *reference* to the users subgraph's `User` key, not a foreign
-- key: orders owns a separate database and resolves the user through federation.
-- `total` is stored in minor units (cents) so money never goes through a float.
CREATE TABLE "orders" (
    "id" BIGSERIAL PRIMARY KEY,
    "userId" BIGINT NOT NULL,
    "total" BIGINT NOT NULL CHECK ("total" >= 0),
    "status" TEXT NOT NULL
        CHECK ("status" IN ('pending', 'paid', 'shipped', 'cancelled')),
    "createdAt" TIMESTAMP(3) NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- The reference column is the access path a future "orders by user" query uses.
CREATE INDEX "orders_userId_idx" ON "orders" ("userId");
