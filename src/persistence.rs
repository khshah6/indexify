use anyhow::Result;
use entity::index::Entity as IndexEntity;
use entity::index::Model as IndexModel;
use sea_orm::ColumnTrait;
use sea_orm::QueryFilter;
use sea_orm::{
    ActiveValue::NotSet, Database, DatabaseConnection, DbErr, EntityTrait, Set, TransactionTrait,
};
use thiserror::Error;

use crate::entity;
use crate::entity::index;
use crate::vectordbs::{self, CreateIndexParams};

#[derive(Debug, Error)]
pub enum RespositoryError {
    #[error(transparent)]
    DatabaseError(#[from] DbErr),

    #[error(transparent)]
    VectorDb(#[from] vectordbs::VectorDbError),

    #[error("index `{0}` not found")]
    IndexNotFound(String),

    #[error("index `{0}` already exists")]
    IndexAlreadyExists(String),

    #[error("unable to serialize unique params `{0}`")]
    UniqueParamsSerializationError(#[from] serde_json::Error),
}

pub struct Respository {
    conn: DatabaseConnection,
}

impl Respository {
    pub async fn new(db_url: &str) -> Result<Self, RespositoryError> {
        let db = Database::connect(db_url).await?;
        Ok(Self { conn: db })
    }

    pub fn new_with_db(db: DatabaseConnection) -> Self {
        Self { conn: db }
    }

    pub async fn create_index(
        &self,
        embedding_model: String,
        index_params: CreateIndexParams,
        vectordb: vectordbs::VectorDBTS,
        text_splitter: String,
    ) -> Result<(), RespositoryError> {
        let mut unique_params = None;
        if let Some(u_params) = &index_params.unique_params {
            unique_params.replace(serde_json::to_string(u_params)?);
        }
        let index = entity::index::ActiveModel {
            name: Set(index_params.name.clone()),
            embedding_model: Set(embedding_model),
            text_splitter: Set(text_splitter),
            vector_db: Set(vectordb.name()),
            vector_db_params: NotSet,
            unique_params: Set(unique_params),
        };
        let tx = self.conn.begin().await?;
        let insert_result = IndexEntity::insert(index).exec(&tx).await;
        if let Err(db_err) = insert_result {
            // TODO Remvoe this hack and drop down to the underlying sqlx error
            // and check if the error is due to primary key violation
            if db_err.to_string().contains("code: 1555") {
                tx.rollback().await?;
                return Err(RespositoryError::IndexAlreadyExists(index_params.name));
            }
        }
        if let Err(err) = vectordb.create_index(index_params.clone()).await {
            tx.rollback().await?;
            return Err(RespositoryError::VectorDb(err));
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn get_index(&self, index: String) -> Result<IndexModel, RespositoryError> {
        let result = IndexEntity::find()
            .filter(index::Column::Name.eq(&index))
            .one(&self.conn)
            .await?
            .ok_or(RespositoryError::IndexNotFound(index));
        result
    }
}
