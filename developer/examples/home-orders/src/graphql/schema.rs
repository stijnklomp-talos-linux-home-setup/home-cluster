//! The federated `orders` schema: an `Order` entity resolved through a
//! [`DataLoader`], and a `User` **reference** owned by the users subgraph.
//!
//! `Order.user` returns only the federation representation
//! (`{ __typename: "User", id: userId }`); the users subgraph resolves the
//! remaining fields. The local `User` type is declared
//! `#[graphql(unresolvable)]`, so async-graphql emits
//! `type User @key(fields: "id", resolvable: false) { id: ID! }`: orders can
//! reference the entity but never claims to resolve it.
//!
//! `Order` is a real entity (`#[graphql(entity)]`), so `_entities` with Order
//! representations is resolved here, batched per request by [`OrderLoader`].
//!
//! Authorization is enforced twice: Hive denies the root fields that carry
//! `@requiresScopes` during validation, and these resolvers re-check the
//! caller because the subgraph is also reachable directly with a valid token.
//! `orders` returns only the caller's rows unless the token has
//! `orders:admin`; `order(id:)` and the `Order` entity deny a different owner.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_graphql::dataloader::{DataLoader, Loader};
use async_graphql::{
    ComplexObject, Context, EmptyMutation, EmptySubscription, ID, Object, Schema, SimpleObject,
};

use super::auth::AuthContext;
use crate::models::order::Order as OrderRow;
use crate::services::orders::OrdersService;

/// Scope required to read orders.
pub const ORDERS_READ: &str = "orders:read";
/// Scope that lifts the owner filter and admits every order.
pub const ORDERS_ADMIN: &str = "orders:admin";

/// Schema type served by [`crate::graphql::GraphQLState`].
pub type ServiceSchema = Schema<Query, EmptyMutation, EmptySubscription>;

/// The federated `Order` entity (`@key(fields: "id")`).
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(rename_fields = "camelCase", complex)]
pub struct Order {
    /// The federation key.
    pub id: ID,
    /// Reference to the users subgraph's `User` entity.
    pub user_id: ID,
    /// Total in minor units (cents).
    pub total: i64,
    /// Order status (`pending`, `paid`, `shipped` or `cancelled`).
    pub status: String,
    /// Creation timestamp (ISO 8601, no timezone suffix).
    pub created_at: String,
}

impl From<OrderRow> for Order {
    fn from(row: OrderRow) -> Self {
        Self {
            id: ID::from(row.id),
            user_id: ID::from(row.user_id),
            total: row.total,
            status: row.status,
            created_at: row.created_at.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
        }
    }
}

#[ComplexObject]
impl Order {
    /// The user who placed the order: a federation reference only. The users
    /// subgraph resolves `name`/`email` for this representation.
    async fn user(&self) -> User {
        User {
            id: self.user_id.clone(),
        }
    }
}

/// Reference to the `User` entity owned by the users subgraph. It carries only
/// the key and is not resolvable here (`resolvable: false` in the SDL).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct User {
    /// The users subgraph's federation key.
    pub id: ID,
}

#[Object(unresolvable)]
impl User {
    /// The federation key.
    async fn id(&self) -> &ID {
        &self.id
    }
}

/// Root query.
pub struct Query;

#[Object]
impl Query {
    /// A page of orders: the caller's unless the token has `orders:admin`.
    ///
    /// Two `requires_scopes` values are two OR groups:
    /// `@requiresScopes(scopes: [["orders:read"], ["orders:admin"]])`. A single
    /// space-separated value would emit one AND group, so keep them separate.
    #[graphql(requires_scopes = "orders:read", requires_scopes = "orders:admin")]
    async fn orders(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 1)] page: u32,
        #[graphql(name = "perPage", default = 100)] per_page: u32,
    ) -> async_graphql::Result<Vec<Order>> {
        let auth = ctx.data::<AuthContext>()?;
        let admin = auth.has_scope(ORDERS_ADMIN);
        if !admin && !auth.has_scope(ORDERS_READ) {
            return Err(async_graphql::Error::new(
                "forbidden: orders:read scope required",
            ));
        }

        let service = ctx.data::<Arc<OrdersService>>()?;
        let query = crate::models::order::GetOrdersQuery { page, per_page };
        let orders = if admin {
            service.get_orders(&query).await
        } else {
            let user_id = auth.user_id.ok_or_else(|| {
                async_graphql::Error::new("forbidden: token has no local user identity")
            })?;
            service.get_orders_for_user(user_id, &query).await
        }
        .map_err(|_| async_graphql::Error::new("failed to load orders"))?;

        Ok(orders.into_iter().map(Order::from).collect())
    }

    /// A single order by id; a different owner is denied.
    #[graphql(requires_scopes = "orders:read", requires_scopes = "orders:admin")]
    async fn order(&self, ctx: &Context<'_>, id: ID) -> async_graphql::Result<Option<Order>> {
        let auth = ctx.data::<AuthContext>()?;
        let Some(order) = load_order(ctx, &id).await? else {
            return Ok(None);
        };
        Ok(can_read_order(auth, &order).then_some(order))
    }

    /// Federation reference resolver: `type Order @key(fields: "id")`.
    ///
    /// The same owner check as `order(id:)`: a representation for somebody
    /// else's order resolves to null instead of leaking its fields.
    #[graphql(
        entity,
        requires_scopes = "orders:read",
        requires_scopes = "orders:admin"
    )]
    async fn find_order_by_id(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> async_graphql::Result<Option<Order>> {
        let auth = ctx.data::<AuthContext>()?;
        let Some(order) = load_order(ctx, &id).await? else {
            return Ok(None);
        };
        Ok(can_read_order(auth, &order).then_some(order))
    }
}

/// `orders:admin`, or the order belongs to the caller.
fn can_read_order(auth: &AuthContext, order: &Order) -> bool {
    auth.has_scope(ORDERS_ADMIN)
        || order
            .user_id
            .as_str()
            .parse::<i64>()
            .ok()
            .is_some_and(|user_id| auth.is_self_id(user_id))
}

async fn load_order(ctx: &Context<'_>, id: &ID) -> async_graphql::Result<Option<Order>> {
    let Some(key) = id.as_str().parse::<i64>().ok() else {
        return Ok(None);
    };
    let loader = ctx.data::<DataLoader<OrderLoader>>()?;
    Ok(loader.load_one(key).await.ok().flatten())
}

/// Builds the schema. Federation support turns on because the query has an
/// entity resolver; `.enable_federation()` makes that explicit.
pub fn schema(orders: Arc<OrdersService>) -> ServiceSchema {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .data(orders)
        .enable_federation()
        .finish()
}

/// Batches order lookups for one request: one repository query per batch of
/// `_entities` representations.
pub struct OrderLoader {
    orders: Arc<OrdersService>,
    stats: Arc<LoadStats>,
}

/// Observability hook used by the batching test.
#[derive(Debug, Default)]
pub struct LoadStats {
    loads: AtomicUsize,
    keys: AtomicUsize,
}

impl LoadStats {
    /// Number of `Loader::load` calls.
    pub fn loads(&self) -> usize {
        self.loads.load(Ordering::Relaxed)
    }

    /// Total keys passed to those calls.
    pub fn keys(&self) -> usize {
        self.keys.load(Ordering::Relaxed)
    }
}

impl OrderLoader {
    /// Creates a loader over the orders service.
    pub fn new(orders: Arc<OrdersService>) -> Self {
        Self {
            orders,
            stats: Arc::default(),
        }
    }

    /// Handle to this loader's call counters.
    pub fn stats(&self) -> Arc<LoadStats> {
        Arc::clone(&self.stats)
    }
}

impl Loader<i64> for OrderLoader {
    type Value = Order;
    type Error = String;

    fn load(
        &self,
        keys: &[i64],
    ) -> impl Future<Output = Result<HashMap<i64, Self::Value>, Self::Error>> + Send {
        self.stats.loads.fetch_add(1, Ordering::Relaxed);
        self.stats.keys.fetch_add(keys.len(), Ordering::Relaxed);
        let orders = Arc::clone(&self.orders);
        let keys = keys.to_vec();

        async move {
            let rows = orders.find_by_ids(&keys).await.map_err(|error| {
                tracing::error!(error = %error, "GraphQL order batch load failed");
                "failed to load orders".to_owned()
            })?;
            Ok(rows
                .into_iter()
                .map(|row| (row.id, Order::from(row)))
                .collect())
        }
    }
}
