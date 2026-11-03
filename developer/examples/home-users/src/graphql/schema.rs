//! The federated `users` schema: a `User` entity resolved through a
//! [`DataLoader`], plus the authenticated `viewer` query.
//!
//! The `User` entity and its `@key(fields: "id")` come from the
//! `#[graphql(entity)]` resolver; async-graphql generates `_service` and
//! `_entities` and the Federation v2 `@link` directive.
//!
//! Authorization is enforced twice: Hive denies fields that carry
//! `@requiresScopes` (coarse scopes) during validation, and the resolvers here
//! re-check the caller because the subgraph is also reachable directly with a
//! valid token. `users` requires `users:read`; `User.email` and `user(id:)`
//! are visible to the caller's own (verified) record or with `users:read`.
//! `User.email` deliberately has no `@requiresScopes`: Hive cannot express
//! "self or scope", so the resolver is the single guard and the gateway lets
//! the field through for the service to decide. `viewer.email` is always the
//! caller's own address.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_graphql::dataloader::{DataLoader, Loader};
use async_graphql::{
    ComplexObject, Context, EmptyMutation, EmptySubscription, ID, Object, Schema, SimpleObject,
};

use super::auth::AuthContext;
use crate::models::user::User as UserRow;
use crate::services::users::UsersService;

/// Scope required to list users and read another user's email.
pub const USERS_READ: &str = "users:read";

/// Schema type served by [`crate::graphql::GraphQLState`].
pub type ServiceSchema = Schema<Query, EmptyMutation, EmptySubscription>;

/// The federated `User` entity (`@key(fields: "id")`).
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(rename_fields = "camelCase", complex)]
pub struct User {
    /// The federation key.
    pub id: ID,
    /// Contact address, guarded by [`ComplexObject::email`].
    #[graphql(skip)]
    pub email: String,
    /// Display name.
    pub name: String,
    /// Creation timestamp (ISO 8601, no timezone suffix).
    pub created_at: String,
}

#[ComplexObject]
impl User {
    /// Initials derived from the display name: "Ada Lovelace" -> "AL".
    ///
    /// A computed field, not a column: it only exists in the SDL, so a change
    /// to it is a pure schema change that the gateway can pick up without a
    /// subgraph image rebuild of the gateway or a Hive restart.
    async fn initials(&self) -> String {
        let mut words = self.name.split_whitespace();
        let first = words.next().and_then(|word| word.chars().next());
        let last = words.last().and_then(|word| word.chars().next());
        [first, last]
            .into_iter()
            .flatten()
            .flat_map(char::to_uppercase)
            .collect()
    }

    /// Contact address: the caller's own verified record, or any user with
    /// `users:read`. No `@requiresScopes` here: the gateway cannot express
    /// "self or scope", so this resolver is the guard (and still applies to
    /// direct subgraph calls).
    async fn email(&self, ctx: &Context<'_>) -> async_graphql::Result<String> {
        let auth = ctx.data::<AuthContext>()?;
        if auth.has_scope(USERS_READ) {
            return Ok(self.email.clone());
        }
        let Ok(id) = self.id.as_str().parse::<i64>() else {
            return Err(forbidden_email());
        };
        if auth.is_self(id, &self.email) {
            Ok(self.email.clone())
        } else {
            Err(forbidden_email())
        }
    }
}

impl From<UserRow> for User {
    fn from(row: UserRow) -> Self {
        Self {
            id: ID::from(row.id),
            email: row.email,
            name: row.name,
            created_at: row.created_at.format("%Y-%m-%dT%H:%M:%S%.f").to_string(),
        }
    }
}

/// The error a caller without `users:read` gets for someone else's email.
fn forbidden_email() -> async_graphql::Error {
    async_graphql::Error::new("forbidden: users:read scope required to read another user's email")
}

/// The authenticated caller, resolved from the JWT context.
#[derive(Debug, Clone, PartialEq, Eq, SimpleObject)]
#[graphql(rename_fields = "camelCase")]
pub struct Viewer {
    /// Token `sub` claim.
    pub subject: Option<String>,
    /// Token `email` claim (the caller's own address).
    pub email: Option<String>,
    /// Whether the token's email claim is verified.
    pub email_verified: bool,
    /// Local user id from `user_id`/`custom:user_id` or a numeric `sub`.
    pub user_id: Option<i64>,
    /// Token scopes.
    pub scopes: Vec<String>,
}

/// Root query.
pub struct Query;

#[Object]
impl Query {
    /// The caller authenticated by the bearer token.
    async fn viewer(&self, ctx: &Context<'_>) -> async_graphql::Result<Viewer> {
        let auth = ctx.data::<AuthContext>()?;
        Ok(Viewer {
            subject: auth.subject.clone(),
            email: auth.email.clone(),
            email_verified: auth.email_verified,
            user_id: auth.user_id,
            scopes: auth.scopes.clone(),
        })
    }

    /// A page of users, ordered by id.
    #[graphql(requires_scopes = "users:read")]
    async fn users(
        &self,
        ctx: &Context<'_>,
        #[graphql(default = 1)] page: u32,
        #[graphql(name = "perPage", default = 100)] per_page: u32,
    ) -> async_graphql::Result<Vec<User>> {
        let auth = ctx.data::<AuthContext>()?;
        if !auth.has_scope(USERS_READ) {
            return Err(async_graphql::Error::new(
                "forbidden: users:read scope required",
            ));
        }
        let service = ctx.data::<Arc<UsersService>>()?;
        let query = crate::models::user::GetUsersQuery { page, per_page };
        let users = service
            .get_users(&query)
            .await
            .map_err(|_| async_graphql::Error::new("failed to load users"))?;
        Ok(users.into_iter().map(User::from).collect())
    }

    /// A single user by id: visible to self or with `users:read`.
    async fn user(&self, ctx: &Context<'_>, id: ID) -> async_graphql::Result<Option<User>> {
        let auth = ctx.data::<AuthContext>()?;
        let Some(user) = load_user(ctx, &id).await? else {
            return Ok(None);
        };
        let key = id.as_str().parse::<i64>().ok();
        let visible =
            auth.has_scope(USERS_READ) || key.is_some_and(|key| auth.is_self(key, &user.email));
        Ok(visible.then_some(user))
    }

    /// Federation reference resolver: `type User @key(fields: "id")`.
    ///
    /// The entity is returned without an ownership check: the entity is the
    /// only way the orders subgraph can resolve `Order.user.name` for orders
    /// the caller does own, and the sensitive `email` field has its own guard.
    #[graphql(entity)]
    async fn find_user_by_id(
        &self,
        ctx: &Context<'_>,
        id: ID,
    ) -> async_graphql::Result<Option<User>> {
        load_user(ctx, &id).await
    }
}

async fn load_user(ctx: &Context<'_>, id: &ID) -> async_graphql::Result<Option<User>> {
    let Some(key) = id.as_str().parse::<i64>().ok() else {
        return Ok(None);
    };
    let loader = ctx.data::<DataLoader<UserLoader>>()?;
    Ok(loader.load_one(key).await.ok().flatten())
}

/// Builds the schema. Federation support turns on because the query has an
/// entity resolver; `.enable_federation()` makes that explicit.
pub fn schema(users: Arc<UsersService>) -> ServiceSchema {
    Schema::build(Query, EmptyMutation, EmptySubscription)
        .data(users)
        .enable_federation()
        .finish()
}

/// Batches user lookups for one request: one repository query per batch of
/// `_entities` representations.
pub struct UserLoader {
    users: Arc<UsersService>,
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

impl UserLoader {
    /// Creates a loader over the users service.
    pub fn new(users: Arc<UsersService>) -> Self {
        Self {
            users,
            stats: Arc::default(),
        }
    }

    /// Handle to this loader's call counters.
    pub fn stats(&self) -> Arc<LoadStats> {
        Arc::clone(&self.stats)
    }
}

impl Loader<i64> for UserLoader {
    type Value = User;
    type Error = String;

    fn load(
        &self,
        keys: &[i64],
    ) -> impl Future<Output = Result<HashMap<i64, Self::Value>, Self::Error>> + Send {
        self.stats.loads.fetch_add(1, Ordering::Relaxed);
        self.stats.keys.fetch_add(keys.len(), Ordering::Relaxed);
        let users = Arc::clone(&self.users);
        let keys = keys.to_vec();

        async move {
            let rows = users.find_by_ids(&keys).await.map_err(|error| {
                tracing::error!(error = %error, "GraphQL user batch load failed");
                "failed to load users".to_owned()
            })?;
            Ok(rows
                .into_iter()
                .map(|row| (row.id, User::from(row)))
                .collect())
        }
    }
}
