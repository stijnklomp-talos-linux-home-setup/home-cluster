//! Users service: the pass-through layer between the REST controllers, the
//! GraphQL schema and the users repository. Storage errors become problem+json
//! 500s; the source is logged, never exposed (ADR-0005).

use std::sync::Arc;

use crate::models::user::{GetUsersQuery, User};
use crate::problem::ApiError;
use crate::repositories::users::{RepositoryError, UsersRepository};

/// Users operations over the [`UsersRepository`] seam.
#[derive(Clone)]
pub struct UsersService {
    repository: Arc<dyn UsersRepository>,
}

impl UsersService {
    /// Service over any users repository (Postgres in `main`, fakes in tests).
    pub fn new(repository: Arc<dyn UsersRepository>) -> Self {
        Self { repository }
    }

    /// One page of users.
    pub async fn get_users(&self, query: &GetUsersQuery) -> Result<Vec<User>, ApiError> {
        self.repository
            .find_page(query.page, query.per_page)
            .await
            .map_err(internal_error)
    }

    /// One user, or `None` when the id does not exist.
    pub async fn get_user(&self, id: i64) -> Result<Option<User>, ApiError> {
        self.repository.find_by_id(id).await.map_err(internal_error)
    }

    /// The users with these ids; the GraphQL data loader batches into this.
    pub async fn find_by_ids(&self, ids: &[i64]) -> Result<Vec<User>, ApiError> {
        self.repository
            .find_by_ids(ids)
            .await
            .map_err(internal_error)
    }
}

/// Storage failures are 500s.
fn internal_error(error: RepositoryError) -> ApiError {
    ApiError::Internal {
        source: Box::new(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use axum::http::StatusCode;
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};

    use super::*;
    use crate::repositories::users::RepositoryFuture;

    #[derive(Default)]
    struct FakeUsersRepository {
        pages: Mutex<Vec<(u32, u32)>>,
        ids: Mutex<Vec<i64>>,
        fail: bool,
    }

    impl FakeUsersRepository {
        fn failing() -> Self {
            Self {
                fail: true,
                ..Self::default()
            }
        }
    }

    impl UsersRepository for FakeUsersRepository {
        fn find_page(&self, page: u32, per_page: u32) -> RepositoryFuture<'_, Vec<User>> {
            self.pages.lock().unwrap().push((page, per_page));
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(RepositoryError::Database(sqlx::Error::RowNotFound));
                }
                Ok(vec![test_user()])
            })
        }

        fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<User>> {
            self.ids.lock().unwrap().push(id);
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(RepositoryError::Database(sqlx::Error::RowNotFound));
                }
                let mut user = test_user();
                user.id = id;
                Ok(Some(user))
            })
        }

        fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<User>> {
            self.ids.lock().unwrap().extend_from_slice(ids);
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(RepositoryError::Database(sqlx::Error::RowNotFound));
                }
                Ok(vec![test_user()])
            })
        }
    }

    fn test_user() -> User {
        let at = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            NaiveTime::from_hms_micro_opt(12, 0, 0, 123_456).unwrap(),
        );
        User {
            id: 7,
            email: "ada@example.test".to_owned(),
            name: "Ada Lovelace".to_owned(),
            created_at: at,
        }
    }

    fn service(repository: &Arc<FakeUsersRepository>) -> UsersService {
        UsersService::new(repository.clone())
    }

    #[tokio::test]
    async fn get_users_forwards_the_page_and_returns_the_rows() {
        let repository = Arc::new(FakeUsersRepository::default());

        let users = service(&repository)
            .get_users(&GetUsersQuery {
                page: 3,
                per_page: 25,
            })
            .await
            .unwrap();

        assert_eq!(*repository.pages.lock().unwrap(), vec![(3, 25)]);
        assert_eq!(users, vec![test_user()]);
    }

    #[tokio::test]
    async fn get_user_forwards_the_id() {
        let repository = Arc::new(FakeUsersRepository::default());

        let user = service(&repository).get_user(42).await.unwrap().unwrap();

        assert_eq!(*repository.ids.lock().unwrap(), vec![42]);
        assert_eq!(user.id, 42);
    }

    #[tokio::test]
    async fn find_by_ids_forwards_every_key() {
        let repository = Arc::new(FakeUsersRepository::default());

        let users = service(&repository).find_by_ids(&[1, 2]).await.unwrap();

        assert_eq!(*repository.ids.lock().unwrap(), vec![1, 2]);
        assert_eq!(users, vec![test_user()]);
    }

    #[tokio::test]
    async fn repository_failures_become_internal_errors() {
        let repository = Arc::new(FakeUsersRepository::failing());
        let service = service(&repository);

        let error = service
            .get_users(&GetUsersQuery {
                page: 1,
                per_page: 10,
            })
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let error = service.get_user(1).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let error = service.find_by_ids(&[1]).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
