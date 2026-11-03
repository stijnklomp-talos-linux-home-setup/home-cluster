//! Orders service: the pass-through layer between the REST controllers, the
//! GraphQL schema and the orders repository. Storage errors become problem+json
//! 500s; the source is logged, never exposed (ADR-0005).

use std::sync::Arc;

use crate::models::order::{GetOrdersQuery, Order};
use crate::problem::ApiError;
use crate::repositories::orders::{OrdersRepository, RepositoryError};

/// Orders operations over the [`OrdersRepository`] seam.
#[derive(Clone)]
pub struct OrdersService {
    repository: Arc<dyn OrdersRepository>,
}

impl OrdersService {
    /// Service over any orders repository (Postgres in `main`, fakes in tests).
    pub fn new(repository: Arc<dyn OrdersRepository>) -> Self {
        Self { repository }
    }

    /// One page of orders.
    pub async fn get_orders(&self, query: &GetOrdersQuery) -> Result<Vec<Order>, ApiError> {
        self.repository
            .find_page(query.page, query.per_page)
            .await
            .map_err(internal_error)
    }

    /// One page of one user's orders.
    pub async fn get_orders_for_user(
        &self,
        user_id: i64,
        query: &GetOrdersQuery,
    ) -> Result<Vec<Order>, ApiError> {
        self.repository
            .find_page_by_user(user_id, query.page, query.per_page)
            .await
            .map_err(internal_error)
    }

    /// One order, or `None` when the id does not exist.
    pub async fn get_order(&self, id: i64) -> Result<Option<Order>, ApiError> {
        self.repository.find_by_id(id).await.map_err(internal_error)
    }

    /// The orders with these ids; the GraphQL data loader batches into this.
    pub async fn find_by_ids(&self, ids: &[i64]) -> Result<Vec<Order>, ApiError> {
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
    use crate::repositories::orders::RepositoryFuture;

    #[derive(Default)]
    struct FakeOrdersRepository {
        pages: Mutex<Vec<(u32, u32)>>,
        ids: Mutex<Vec<i64>>,
        fail: bool,
    }

    impl FakeOrdersRepository {
        fn failing() -> Self {
            Self {
                fail: true,
                ..Self::default()
            }
        }
    }

    impl OrdersRepository for FakeOrdersRepository {
        fn find_page(&self, page: u32, per_page: u32) -> RepositoryFuture<'_, Vec<Order>> {
            self.pages.lock().unwrap().push((page, per_page));
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(RepositoryError::Database(sqlx::Error::RowNotFound));
                }
                Ok(vec![test_order()])
            })
        }

        fn find_by_id(&self, id: i64) -> RepositoryFuture<'_, Option<Order>> {
            self.ids.lock().unwrap().push(id);
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(RepositoryError::Database(sqlx::Error::RowNotFound));
                }
                let mut order = test_order();
                order.id = id;
                Ok(Some(order))
            })
        }

        fn find_by_ids(&self, ids: &[i64]) -> RepositoryFuture<'_, Vec<Order>> {
            self.ids.lock().unwrap().extend_from_slice(ids);
            let fail = self.fail;
            Box::pin(async move {
                if fail {
                    return Err(RepositoryError::Database(sqlx::Error::RowNotFound));
                }
                Ok(vec![test_order()])
            })
        }
    }

    fn test_order() -> Order {
        let at = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 11, 1).unwrap(),
            NaiveTime::from_hms_micro_opt(12, 0, 0, 123_456).unwrap(),
        );
        Order {
            id: 7,
            user_id: 2,
            total: 12_500,
            status: "paid".to_owned(),
            created_at: at,
        }
    }

    fn service(repository: &Arc<FakeOrdersRepository>) -> OrdersService {
        OrdersService::new(repository.clone())
    }

    #[tokio::test]
    async fn get_orders_forwards_the_page_and_returns_the_rows() {
        let repository = Arc::new(FakeOrdersRepository::default());

        let orders = service(&repository)
            .get_orders(&GetOrdersQuery {
                page: 3,
                per_page: 25,
            })
            .await
            .unwrap();

        assert_eq!(*repository.pages.lock().unwrap(), vec![(3, 25)]);
        assert_eq!(orders, vec![test_order()]);
    }

    #[tokio::test]
    async fn get_order_forwards_the_id() {
        let repository = Arc::new(FakeOrdersRepository::default());

        let order = service(&repository).get_order(42).await.unwrap().unwrap();

        assert_eq!(*repository.ids.lock().unwrap(), vec![42]);
        assert_eq!(order.id, 42);
    }

    #[tokio::test]
    async fn find_by_ids_forwards_every_key() {
        let repository = Arc::new(FakeOrdersRepository::default());

        let orders = service(&repository).find_by_ids(&[1, 2]).await.unwrap();

        assert_eq!(*repository.ids.lock().unwrap(), vec![1, 2]);
        assert_eq!(orders, vec![test_order()]);
    }

    #[tokio::test]
    async fn repository_failures_become_internal_errors() {
        let repository = Arc::new(FakeOrdersRepository::failing());
        let service = service(&repository);

        let error = service
            .get_orders(&GetOrdersQuery {
                page: 1,
                per_page: 10,
            })
            .await
            .unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let error = service.get_order(1).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);

        let error = service.find_by_ids(&[1]).await.unwrap_err();
        assert_eq!(error.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}
