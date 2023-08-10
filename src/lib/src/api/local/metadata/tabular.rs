//! Helper functions to get metadata from tabular files.
//!

use crate::core::df::tabular;
use crate::error::OxenError;
use crate::model::metadata::MetadataTabular;
use crate::opts::DFOpts;

use std::path::Path;

/// Detects the tabular metadata for the given file.
pub fn get_metadata(path: impl AsRef<Path>) -> Result<MetadataTabular, OxenError> {
    let path = path.as_ref();
    let opts = DFOpts::empty();
    let df = tabular::read_df(path, opts)?;
    Ok(MetadataTabular::new(df.width(), df.height()))
}

#[cfg(test)]
mod tests {
    use crate::api;
    use crate::model::metadata::generic_metadata::GenericMetadata;
    use crate::model::metadata::MetadataTabular;
    use crate::model::EntryDataType;
    use crate::test;

    #[test]
    fn test_get_metadata_tabular() {
        let file = test::test_text_file_with_name("celeb_a_200k.csv");
        let metadata = api::local::metadata::get(file).unwrap();

        assert_eq!(metadata.size, 9604701);
        assert_eq!(metadata.data_type, EntryDataType::Tabular);
        assert_eq!(metadata.mime_type, "text/plain");

        let metadata: MetadataTabular = match metadata.metadata.unwrap() {
            GenericMetadata::MetadataTabular(metadata) => metadata,
            _ => panic!("Wrong metadata type"),
        };

        assert_eq!(metadata.tabular.width, 11);
        assert_eq!(metadata.tabular.height, 200_000);
    }
}
