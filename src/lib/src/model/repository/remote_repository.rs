use crate::model::LocalRepository;
use crate::view::RepositoryView;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug, Clone)]
pub struct RemoteRepository {
    pub namespace: String,
    pub name: String,
    pub url: String,
}

impl RemoteRepository {
    pub fn from_view(repository: &RepositoryView, url: &str) -> RemoteRepository {
        RemoteRepository {
            namespace: repository.namespace.clone(),
            name: repository.name.clone(),
            url: String::from(url),
        }
    }

    pub fn from_local(repository: &LocalRepository, url: &str) -> RemoteRepository {
        RemoteRepository {
            namespace: repository.namespace.clone(),
            name: repository.name.clone(),
            url: String::from(url),
        }
    }
}
