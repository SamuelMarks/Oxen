use crate::constants::{CACHE_DIR, COMPARES_DIR, LEFT_COMPARE_COMMIT, RIGHT_COMPARE_COMMIT};
use crate::core::df::tabular::{self};
use crate::error::OxenError;
use crate::model::entry::commit_entry::CompareEntry;
use crate::model::{CommitEntry, DataFrameSize, LocalRepository, Schema};
use crate::opts::DFOpts;

use crate::view::compare::{
    CompareDerivedDF, CompareDupes, CompareResult, CompareSourceDF, CompareTabular,
    CompareTabularRaw,
};
use crate::view::schema::SchemaWithPath;
use crate::{api, util};

use polars::prelude::DataFrame;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub mod hash_compare;
pub mod join_compare;
pub mod utf8_compare;

pub enum CompareStrategy {
    Hash,
    Join,
}

const LEFT: &str = "left";
const RIGHT: &str = "right";
const MATCH: &str = "match";
const DIFF: &str = "diff";
const LEFT_ONLY: &str = "left_only";
const RIGHT_ONLY: &str = "right_only";
const TARGETS_HASH_COL: &str = "_targets_hash";
const KEYS_HASH_COL: &str = "_keys_hash";
const DUPES_PATH: &str = "dupes.json";

#[allow(clippy::too_many_arguments)]
pub fn compare_files(
    strategy: CompareStrategy,
    repo: &LocalRepository,
    compare_id: Option<&str>,
    compare_entry_1: CompareEntry,
    compare_entry_2: CompareEntry,
    keys: Vec<String>,
    targets: Vec<String>,
    output: Option<PathBuf>,
) -> Result<CompareResult, OxenError> {
    // Assert that the files exist in their respective commits and are tabular.
    let file_1 = get_version_file(repo, &compare_entry_1)?;
    let file_2 = get_version_file(repo, &compare_entry_2)?;

    if is_files_tabular(&file_1, &file_2) {
        let result = compare_tabular(
            &file_1,
            &file_2,
            strategy,
            compare_entry_1,
            compare_entry_2,
            repo,
            compare_id,
            keys,
            targets,
            output,
        )?;

        Ok(CompareResult::Tabular(result))
    } else if is_files_utf8(&file_1, &file_2) {
        let result = utf8_compare::compare(&file_1, &file_2)?;
        println!("{result}");

        Ok(CompareResult::Text(result))
    } else {
        Err(OxenError::invalid_file_type(format!(
            "Compare not supported for files, found {:?} and {:?}",
            compare_entry_1.path, compare_entry_2.path
        )))
    }
}

#[allow(clippy::too_many_arguments)]
fn compare_tabular(
    file_1: &Path,
    file_2: &Path,
    strategy: CompareStrategy,
    compare_entry_1: CompareEntry,
    compare_entry_2: CompareEntry,
    repo: &LocalRepository,
    compare_id: Option<&str>,
    keys: Vec<String>,
    targets: Vec<String>,
    output: Option<PathBuf>,
) -> Result<CompareTabular, OxenError> {
    let df_1 = tabular::read_df(file_1, DFOpts::empty())?;
    let df_2 = tabular::read_df(file_2, DFOpts::empty())?;

    let schema_1 = Schema::from_polars(&df_1.schema());
    let schema_2 = Schema::from_polars(&df_2.schema());

    validate_required_fields(schema_1, schema_2, keys.clone(), targets.clone())?;

    let keys = keys.iter().map(|key| key.as_str()).collect::<Vec<&str>>();
    let targets = targets
        .iter()
        .map(|target| target.as_str())
        .collect::<Vec<&str>>();

    let mut compare_tabular_raw = compute_row_comparison(&strategy, &df_1, &df_2, &keys, &targets)?;

    let compare = build_compare_tabular(
        &df_1,
        &df_2,
        &compare_entry_1,
        &compare_entry_2,
        &compare_tabular_raw,
        compare_id,
    );

    maybe_print_compare_output(&strategy, &compare_tabular_raw);
    maybe_save_compare_output(&strategy, &mut compare_tabular_raw, output)?;
    maybe_write_cache(
        repo,
        compare_id,
        compare_entry_1,
        compare_entry_2,
        &mut compare_tabular_raw,
    )?;

    Ok(compare)
}

pub fn get_cached_compare(
    repo: &LocalRepository,
    compare_id: &str,
    compare_entry_1: CompareEntry,
    compare_entry_2: CompareEntry,
) -> Result<Option<CompareTabular>, OxenError> {
    // Check if commits have cahnged since LEFT and RIGHT files were last cached
    let (cached_left_id, cached_right_id) = get_compare_commit_ids(repo, compare_id)?;

    // If commits cache files do not exist or have changed since last hash (via branch name) then return None to recompute
    if cached_left_id.is_none() || cached_right_id.is_none() {
        return Ok(None);
    }

    if compare_entry_1.commit_entry.is_none() || compare_entry_2.commit_entry.is_none() {
        return Ok(None);
    }

    let left_commit = compare_entry_1.commit_entry.unwrap();
    let right_commit = compare_entry_2.commit_entry.unwrap();

    if cached_left_id.unwrap() != left_commit.commit_id
        || cached_right_id.unwrap() != right_commit.commit_id
    {
        return Ok(None);
    }

    let left_full_df = tabular::read_df(
        api::local::diff::get_version_file_from_commit_id(
            repo,
            &left_commit.commit_id,
            &compare_entry_1.path,
        )?,
        DFOpts::empty(),
    )?;
    let right_full_df = tabular::read_df(
        api::local::diff::get_version_file_from_commit_id(
            repo,
            &right_commit.commit_id,
            &compare_entry_2.path,
        )?,
        DFOpts::empty(),
    )?;

    let left_schema = SchemaWithPath {
        schema: Schema::from_polars(&left_full_df.schema()),
        path: compare_entry_1.path.to_str().map(|s| s.to_owned()).unwrap(),
    };

    let right_schema = SchemaWithPath {
        schema: Schema::from_polars(&right_full_df.schema()),
        path: compare_entry_2.path.to_str().map(|s| s.to_owned()).unwrap(),
    };

    let match_df = tabular::read_df(get_compare_match_path(repo, compare_id), DFOpts::empty())?;
    let diff_df = tabular::read_df(get_compare_diff_path(repo, compare_id), DFOpts::empty())?;
    let left_only_df = tabular::read_df(get_compare_left_path(repo, compare_id), DFOpts::empty())?;
    let right_only_df =
        tabular::read_df(get_compare_right_path(repo, compare_id), DFOpts::empty())?;

    let match_schema = Schema::from_polars(&match_df.schema());
    let diff_schema = Schema::from_polars(&diff_df.schema());
    let left_only_schema = Schema::from_polars(&left_only_df.schema());
    let right_only_schema = Schema::from_polars(&right_only_df.schema());

    let source_df_left = CompareSourceDF::from_name_df_entry_schema(
        LEFT,
        left_full_df,
        &left_commit,
        left_schema.schema.clone(),
    );
    let source_df_right = CompareSourceDF::from_name_df_entry_schema(
        RIGHT,
        right_full_df,
        &right_commit,
        right_schema.schema.clone(),
    );

    let derived_df_match = CompareDerivedDF::from_compare_info(
        MATCH,
        Some(compare_id),
        Some(&left_commit),
        Some(&right_commit),
        match_df,
        match_schema,
    );
    let derived_df_diff = CompareDerivedDF::from_compare_info(
        DIFF,
        Some(compare_id),
        Some(&left_commit),
        Some(&right_commit),
        diff_df,
        diff_schema,
    );
    let derived_df_left_only = CompareDerivedDF::from_compare_info(
        LEFT_ONLY,
        Some(compare_id),
        Some(&left_commit),
        Some(&right_commit),
        left_only_df,
        left_only_schema,
    );
    let derived_df_right_only = CompareDerivedDF::from_compare_info(
        RIGHT_ONLY,
        Some(compare_id),
        Some(&left_commit),
        Some(&right_commit),
        right_only_df,
        right_only_schema,
    );

    let source_dfs: HashMap<String, CompareSourceDF> = HashMap::from([
        (LEFT.to_string(), source_df_left),
        (RIGHT.to_string(), source_df_right),
    ]);

    let derived_dfs: HashMap<String, CompareDerivedDF> = HashMap::from([
        (MATCH.to_string(), derived_df_match),
        (DIFF.to_string(), derived_df_diff),
        (LEFT_ONLY.to_string(), derived_df_left_only),
        (RIGHT_ONLY.to_string(), derived_df_right_only),
    ]);

    let compare_results = CompareTabular {
        source: source_dfs,
        derived: derived_dfs,
        dupes: read_dupes(repo, compare_id)?,
    };

    Ok(Some(compare_results))
}

pub fn get_compare_dir(repo: &LocalRepository, compare_id: &str) -> PathBuf {
    util::fs::oxen_hidden_dir(&repo.path)
        .join(CACHE_DIR)
        .join(COMPARES_DIR)
        .join(compare_id)
}

fn get_compare_match_path(repo: &LocalRepository, compare_id: &str) -> PathBuf {
    let compare_dir = get_compare_dir(repo, compare_id);
    compare_dir.join("match.parquet")
}

fn get_compare_diff_path(repo: &LocalRepository, compare_id: &str) -> PathBuf {
    let compare_dir = get_compare_dir(repo, compare_id);
    compare_dir.join("diff.parquet")
}

fn get_compare_left_path(repo: &LocalRepository, compare_id: &str) -> PathBuf {
    let compare_dir = get_compare_dir(repo, compare_id);
    compare_dir.join("left_only.parquet")
}

fn get_compare_right_path(repo: &LocalRepository, compare_id: &str) -> PathBuf {
    let compare_dir = get_compare_dir(repo, compare_id);
    compare_dir.join("right_only.parquet")
}

fn maybe_write_dupes(
    repo: &LocalRepository,
    compare_id: &str,
    dupes: &CompareDupes,
) -> Result<(), OxenError> {
    let compare_dir = get_compare_dir(repo, compare_id);

    if !compare_dir.exists() {
        std::fs::create_dir_all(&compare_dir)?;
    }

    let dupes_path = compare_dir.join(DUPES_PATH);

    std::fs::write(dupes_path, serde_json::to_string(&dupes)?)?;

    Ok(())
}

fn read_dupes(repo: &LocalRepository, compare_id: &str) -> Result<CompareDupes, OxenError> {
    let compare_dir = get_compare_dir(repo, compare_id);
    let dupes_path = compare_dir.join(DUPES_PATH);

    if !dupes_path.exists() {
        return Ok(CompareDupes::empty());
    }

    let dupes: CompareDupes = serde_json::from_str(&std::fs::read_to_string(dupes_path)?)?;

    Ok(dupes)
}

fn write_compare_dfs(
    repo: &LocalRepository,
    compare_id: &str,
    left_only: &mut DataFrame,
    right_only: &mut DataFrame,
    match_df: &mut DataFrame,
    diff_df: &mut DataFrame,
) -> Result<(), OxenError> {
    let compare_dir = get_compare_dir(repo, compare_id);

    if !compare_dir.exists() {
        std::fs::create_dir_all(&compare_dir)?;
    }

    let match_path = get_compare_match_path(repo, compare_id);
    let diff_path = get_compare_diff_path(repo, compare_id);
    let left_path = get_compare_left_path(repo, compare_id);
    let right_path = get_compare_right_path(repo, compare_id);

    log::debug!("writing {:?} rows to {:?}", match_df.height(), match_path);
    tabular::write_df(match_df, &match_path)?;
    log::debug!("writing {:?} rows to {:?}", diff_df.height(), diff_path);
    tabular::write_df(diff_df, &diff_path)?;
    log::debug!("writing {:?} rows to {:?}", left_only.height(), left_path);
    tabular::write_df(left_only, &left_path)?;
    log::debug!("writing {:?} rows to {:?}", right_only.height(), right_path);
    tabular::write_df(right_only, &right_path)?;

    Ok(())
}

fn write_compare_commit_ids(
    repo: &LocalRepository,
    compare_id: &str,
    left_entry: &Option<CommitEntry>,
    right_entry: &Option<CommitEntry>,
) -> Result<(), OxenError> {
    let compare_dir = get_compare_dir(repo, compare_id);

    if !compare_dir.exists() {
        std::fs::create_dir_all(&compare_dir)?;
    }

    let left_path = compare_dir.join(LEFT_COMPARE_COMMIT);
    let right_path = compare_dir.join(RIGHT_COMPARE_COMMIT);

    if let Some(commit_entry) = left_entry {
        let left_id = &commit_entry.commit_id;
        std::fs::write(left_path, left_id)?;
    }

    if let Some(commit_entry) = right_entry {
        let right_id = &commit_entry.commit_id;
        std::fs::write(right_path, right_id)?;
    }

    Ok(())
}

fn get_compare_commit_ids(
    repo: &LocalRepository,
    compare_id: &str,
) -> Result<(Option<String>, Option<String>), OxenError> {
    let compare_dir = get_compare_dir(repo, compare_id);

    if !compare_dir.exists() {
        return Ok((None, None));
    }

    let left_path = compare_dir.join(LEFT_COMPARE_COMMIT);
    let right_path = compare_dir.join(RIGHT_COMPARE_COMMIT);

    // Should exist together or not at all, but recalculate if for some reaosn one not present
    if !left_path.exists() || !right_path.exists() {
        return Ok((None, None));
    }

    let left_id = std::fs::read_to_string(left_path)?;
    let right_id = std::fs::read_to_string(right_path)?;

    Ok((Some(left_id), Some(right_id)))
}

fn compute_row_comparison(
    strategy: &CompareStrategy,
    df_1: &DataFrame,
    df_2: &DataFrame,
    keys: &[&str],
    targets: &[&str],
) -> Result<CompareTabularRaw, OxenError> {
    let schema_1 = Schema::from_polars(&df_1.schema());
    let schema_2 = Schema::from_polars(&df_2.schema());
    let mut dupes = CompareDupes { left: 0, right: 0 };

    let dataframes = match strategy {
        CompareStrategy::Hash => hash_compare::compare(df_1, df_2, &schema_1, &schema_2)?,
        CompareStrategy::Join => {
            // TODO: unsure if hash comparison or join is faster here - would guess join, could use some testing
            let (df_1, df_2) = hash_dfs(
                df_1.clone(),
                df_2.clone(),
                keys.to_owned(),
                targets.to_owned(),
            )?;
            dupes = CompareDupes {
                left: tabular::n_duped_rows(&df_1, &[KEYS_HASH_COL])?,
                right: tabular::n_duped_rows(&df_2, &[KEYS_HASH_COL])?,
            };

            join_compare::compare(&df_1, &df_2, targets.to_owned(), keys.to_owned())?
        }
    };

    let (diff_df, match_df, left_only_df, right_only_df) = dataframes;

    let compare_tabular_raw = CompareTabularRaw {
        diff_df,
        match_df,
        left_only_df,
        right_only_df,
        dupes,
    };

    Ok(compare_tabular_raw)
}

fn hash_dfs(
    mut left_df: DataFrame,
    mut right_df: DataFrame,
    keys: Vec<&str>,
    targets: Vec<&str>,
) -> Result<(DataFrame, DataFrame), OxenError> {
    // Subset to only targets and keys - also checks that these are present
    let out_fields = keys.iter().chain(targets.iter()).copied();

    left_df = left_df.select(out_fields.clone())?;
    right_df = right_df.select(out_fields)?;

    // Generate hash columns for target set and key set
    left_df = tabular::df_hash_rows_on_cols(left_df, targets.clone(), TARGETS_HASH_COL)?;
    right_df = tabular::df_hash_rows_on_cols(right_df, targets.clone(), TARGETS_HASH_COL)?;

    left_df = tabular::df_hash_rows_on_cols(left_df, keys.clone(), KEYS_HASH_COL)?;
    right_df = tabular::df_hash_rows_on_cols(right_df, keys.clone(), KEYS_HASH_COL)?;
    Ok((left_df, right_df))
}

fn get_version_file(
    repo: &LocalRepository,
    compare_entry: &CompareEntry,
) -> Result<PathBuf, OxenError> {
    if let Some(commit_entry) = &compare_entry.commit_entry {
        api::local::diff::get_version_file_from_commit_id(
            repo,
            &commit_entry.commit_id,
            &commit_entry.path,
        )
    } else {
        Ok(compare_entry.path.clone())
    }
}

fn build_compare_tabular(
    df_1: &DataFrame,
    df_2: &DataFrame,
    compare_entry_1: &CompareEntry,
    compare_entry_2: &CompareEntry,
    compare_tabular_raw: &CompareTabularRaw,
    compare_id: Option<&str>,
) -> CompareTabular {
    let left_only_df = &compare_tabular_raw.left_only_df;
    let right_only_df = &compare_tabular_raw.right_only_df;
    let diff_df = &compare_tabular_raw.diff_df;
    let match_df = &compare_tabular_raw.match_df;

    let diff_schema = Schema::from_polars(&diff_df.schema());
    let match_schema = Schema::from_polars(&match_df.schema());
    let left_only_schema = Schema::from_polars(&left_only_df.schema());
    let right_only_schema = Schema::from_polars(&right_only_df.schema());

    let df_1_size = DataFrameSize::from_df(&df_1);
    let df_2_size = DataFrameSize::from_df(&df_2);
    let og_schema_1 = Schema::from_polars(&df_1.schema());
    let og_schema_2 = Schema::from_polars(&df_2.schema());

    let derived_df_match = CompareDerivedDF::from_compare_info(
        MATCH,
        compare_id,
        compare_entry_1.commit_entry.as_ref(),
        compare_entry_2.commit_entry.as_ref(),
        match_df.clone(),
        match_schema,
    );
    let derived_df_diff = CompareDerivedDF::from_compare_info(
        DIFF,
        compare_id,
        compare_entry_1.commit_entry.as_ref(),
        compare_entry_2.commit_entry.as_ref(),
        diff_df.clone(),
        diff_schema,
    );
    let derived_df_left_only = CompareDerivedDF::from_compare_info(
        LEFT_ONLY,
        compare_id,
        compare_entry_1.commit_entry.as_ref(),
        compare_entry_2.commit_entry.as_ref(),
        left_only_df.clone(),
        left_only_schema,
    );
    let derived_df_right_only = CompareDerivedDF::from_compare_info(
        RIGHT_ONLY,
        compare_id,
        compare_entry_1.commit_entry.as_ref(),
        compare_entry_2.commit_entry.as_ref(),
        right_only_df.clone(),
        right_only_schema,
    );

    let source_df_left = CompareSourceDF {
        name: LEFT.to_string(),
        path: compare_entry_1.path.clone(),
        version: compare_entry_1
            .clone()
            .commit_entry
            .map(|c| c.commit_id)
            .unwrap_or("".to_owned()),
        schema: og_schema_1.clone(),
        size: df_1_size,
    };

    let source_df_right = CompareSourceDF {
        name: RIGHT.to_string(),
        path: compare_entry_2.path.clone(),
        version: compare_entry_2
            .clone()
            .commit_entry
            .map(|c| c.commit_id)
            .unwrap_or("".to_owned()),
        schema: og_schema_2.clone(),
        size: df_2_size,
    };

    let source_dfs: HashMap<String, CompareSourceDF> = HashMap::from([
        (LEFT.to_string(), source_df_left),
        (RIGHT.to_string(), source_df_right),
    ]);

    let derived_dfs: HashMap<String, CompareDerivedDF> = HashMap::from([
        (MATCH.to_string(), derived_df_match),
        (DIFF.to_string(), derived_df_diff),
        (LEFT_ONLY.to_string(), derived_df_left_only),
        (RIGHT_ONLY.to_string(), derived_df_right_only),
    ]);

    CompareTabular {
        source: source_dfs,
        derived: derived_dfs,
        dupes: compare_tabular_raw.dupes.clone(),
    }
}

fn validate_required_fields(
    schema_1: Schema,
    schema_2: Schema,
    keys: Vec<String>,
    targets: Vec<String>,
) -> Result<(), OxenError> {
    // Subset dataframes to "keys" and "targets"
    #[allow(clippy::map_clone)]
    let required_fields = keys
        .iter()
        .chain(targets.iter())
        .cloned()
        .collect::<Vec<String>>();

    // Make sure both dataframes have all required fields

    if !schema_1.has_field_names(&required_fields) {
        return Err(OxenError::incompatible_schemas(required_fields, schema_1));
    };

    if !schema_2.has_field_names(&required_fields) {
        return Err(OxenError::incompatible_schemas(required_fields, schema_2));
    };

    Ok(())
}

fn maybe_write_cache(
    repo: &LocalRepository,
    compare_id: Option<&str>,
    compare_entry_1: CompareEntry,
    compare_entry_2: CompareEntry,
    compare_tabular_raw: &mut CompareTabularRaw,
) -> Result<(), OxenError> {
    if let Some(compare_id) = compare_id {
        write_compare_commit_ids(
            repo,
            compare_id,
            &compare_entry_1.commit_entry,
            &compare_entry_2.commit_entry,
        )?;
        write_compare_dfs(
            repo,
            compare_id,
            &mut compare_tabular_raw.left_only_df,
            &mut compare_tabular_raw.right_only_df,
            &mut compare_tabular_raw.match_df,
            &mut compare_tabular_raw.diff_df,
        )?;
        maybe_write_dupes(repo, compare_id, &compare_tabular_raw.dupes)?;
    }

    Ok(())
}

fn maybe_print_compare_output(strategy: &CompareStrategy, compare_tabular_raw: &CompareTabularRaw) {
    let left_only_df = &compare_tabular_raw.left_only_df;
    let right_only_df = &compare_tabular_raw.right_only_df;
    let diff_df = &compare_tabular_raw.diff_df;
    let match_df = &compare_tabular_raw.match_df;

    match strategy {
        CompareStrategy::Hash => {
            println!("Added rows");
            println!("{:?}", left_only_df);

            println!("Removed rows");
            println!("{:?}", right_only_df);
        }

        CompareStrategy::Join => {
            println!("Rows with matching keys and DIFFERENT targets");
            println!("{:?}", diff_df);

            println!("Rows with matching keys and SAME targets");
            println!("{:?}", match_df);

            println!("Rows with keys only in LEFT DataFrame");
            println!("{:?}", left_only_df);

            println!("Rows with keys only in RIGHT DataFrame");
            println!("{:?}", right_only_df);
        }
    }
}

fn maybe_save_compare_output(
    strategy: &CompareStrategy,
    compare_tabular_raw: &mut CompareTabularRaw,
    output: Option<PathBuf>,
) -> Result<(), OxenError> {
    let left_only_df = &mut compare_tabular_raw.left_only_df;
    let right_only_df = &mut compare_tabular_raw.right_only_df;
    let diff_df = &mut compare_tabular_raw.diff_df;
    let match_df = &mut compare_tabular_raw.match_df;

    let (df_1, file_name_1, df_2, file_name_2) = match strategy {
        CompareStrategy::Hash => (left_only_df, "added.csv", right_only_df, "removed.csv"),
        CompareStrategy::Join => (diff_df, "diff.csv", match_df, "match.csv"),
    };

    // // Save to disk if we have an output - i.e., if called from CLI
    if let Some(output) = output {
        std::fs::create_dir_all(output.clone())?;
        let file_1_path = output.join(file_name_1);
        let file_2_path = output.join(file_name_2);
        tabular::write_df(df_1, file_1_path.clone())?;
        tabular::write_df(df_2, file_2_path.clone())?;
    }

    Ok(())
}

fn is_files_tabular(file_1: &Path, file_2: &Path) -> bool {
    util::fs::is_tabular(file_1) || util::fs::is_tabular(file_2)
}
fn is_files_utf8(file_1: &Path, file_2: &Path) -> bool {
    util::fs::is_utf8(file_1) && util::fs::is_utf8(file_2)
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::path::PathBuf;

    use crate::api;
    use crate::api::local::compare::CompareStrategy;
    use crate::command;
    use crate::core::df::tabular;
    use crate::error::OxenError;
    use crate::model::entry::commit_entry::CompareEntry;
    use crate::opts::DFOpts;
    use crate::test;
    use crate::view::compare::CompareResult;

    #[test]
    fn test_compare_fails_when_not_tabular() -> Result<(), OxenError> {
        test::run_bounding_box_csv_repo_test_fully_committed(|repo| {
            let hello_file = repo.path.join("Hello.txt");
            let world_file = repo.path.join("World.txt");
            test::write_txt_file_to_path(&hello_file, "Hello")?;
            test::write_txt_file_to_path(&world_file, "World")?;

            command::add(&repo, &hello_file)?;
            command::add(&repo, &world_file)?;

            command::commit(&repo, "adding_new_files")?;

            let head_commit = api::local::commits::head_commit(&repo)?;

            let keys = vec![];
            let targets = vec![];

            let entry_left = api::local::entries::get_commit_entry(
                &repo,
                &head_commit,
                &PathBuf::from("Hello.txt"),
            )?
            .unwrap();

            let entry_right = api::local::entries::get_commit_entry(
                &repo,
                &head_commit,
                &PathBuf::from("World.txt"),
            )?
            .unwrap();

            let compare_entry_1 = CompareEntry {
                commit_entry: Some(entry_left),
                path: hello_file,
            };

            let compare_entry_2 = CompareEntry {
                commit_entry: Some(entry_right),
                path: world_file,
            };

            let result = api::local::compare::compare_files(
                CompareStrategy::Hash,
                &repo,
                None,
                compare_entry_1,
                compare_entry_2,
                keys,
                targets,
                None,
            )?;

            // TOOD : FIX TEST
            // assert!(matches!(result.unwrap_err(), OxenError::InvalidFileType(_)));

            Ok(())
        })
    }

    #[test]
    fn test_compare_files() -> Result<(), OxenError> {
        test::run_compare_data_repo_test_fully_commited(|repo| {
            let left_file = PathBuf::from("compare_left.csv");
            let right_file = PathBuf::from("compare_right.csv");
            let head_commit = api::local::commits::head_commit(&repo)?;

            let entry_left =
                api::local::entries::get_commit_entry(&repo, &head_commit, &left_file)?.unwrap();

            let entry_right =
                api::local::entries::get_commit_entry(&repo, &head_commit, &right_file)?.unwrap();

            let compare_entry_1 = CompareEntry {
                commit_entry: Some(entry_left),
                path: left_file,
            };

            let compare_entry_2 = CompareEntry {
                commit_entry: Some(entry_right),
                path: right_file,
            };

            let result = api::local::compare::compare_files(
                CompareStrategy::Join,
                &repo,
                None,
                compare_entry_1,
                compare_entry_2,
                vec![
                    "height".to_string(),
                    "weight".to_string(),
                    "gender".to_string(),
                ],
                vec!["target".to_string(), "other_target".to_string()],
                None,
            )?;

            if let CompareResult::Tabular(compare) = result {
                // Should be updated values
                assert_eq!(compare.derived["left_only"].size.height, 2);
                assert_eq!(compare.derived["right_only"].size.height, 1);
                assert_eq!(compare.derived["match"].size.height, 6);
                assert_eq!(compare.derived["diff"].size.height, 5);
            } else {
                assert_eq!(true, false, "Wrong result type for input files")
            }

            Ok(())
        })
    }

    #[test]
    fn test_compare_cache_miss_when_branch_ref_updates() -> Result<(), OxenError> {
        test::run_compare_data_repo_test_fully_commited(|repo| {
            let old_head = api::local::commits::head_commit(&repo)?;
            let left_file = PathBuf::from("compare_left.csv");
            let right_file = PathBuf::from("compare_right.csv");
            let left_entry =
                api::local::entries::get_commit_entry(&repo, &old_head, &left_file)?.unwrap();
            let right_entry =
                api::local::entries::get_commit_entry(&repo, &old_head, &right_file)?.unwrap();

            let compare_entry_1 = CompareEntry {
                commit_entry: Some(left_entry),
                path: left_file.clone(),
            };

            let compare_entry_2 = CompareEntry {
                commit_entry: Some(right_entry),
                path: right_file.clone(),
            };

            // Create compare on this commit
            api::local::compare::compare_files(
                CompareStrategy::Join,
                &repo,
                Some("a_compare_id"),
                compare_entry_1.clone(),
                compare_entry_2.clone(),
                vec![
                    String::from("height"),
                    String::from("weight"),
                    String::from("gender"),
                ],
                vec![String::from("target"), String::from("other_target")],
                None,
            )?;

            // Check getting via cache
            let compare = api::local::compare::get_cached_compare(
                &repo,
                "a_compare_id",
                compare_entry_1,
                compare_entry_2,
            )?
            .unwrap();

            assert_eq!(compare.derived["left_only"].size.height, 2);
            assert_eq!(compare.derived["right_only"].size.height, 1);
            assert_eq!(compare.derived["match"].size.height, 6);
            assert_eq!(compare.derived["diff"].size.height, 5);

            // Update one of the files
            let path = Path::new("compare_left.csv");
            let file_path = repo.path.join(path);
            let mut df = tabular::read_df(&file_path, DFOpts::empty())?;
            df = df.slice(0, 6);
            tabular::write_df(&mut df, &file_path)?;

            // Commit the new modification
            command::add(&repo, &repo.path)?;
            command::status(&repo)?;
            command::commit(&repo, "updating compare_left.csv")?;

            // Get new entries and check the cached compare
            let new_head = api::local::commits::head_commit(&repo)?;
            let new_left_entry = api::local::entries::get_commit_entry(
                &repo,
                &new_head,
                &PathBuf::from("compare_left.csv"),
            )?
            .unwrap();
            let new_right_entry = api::local::entries::get_commit_entry(
                &repo,
                &new_head,
                &PathBuf::from("compare_right.csv"),
            )?
            .unwrap();

            let new_compare_entry_1 = CompareEntry {
                commit_entry: Some(new_left_entry),
                path: left_file,
            };

            let new_compare_entry_2 = CompareEntry {
                commit_entry: Some(new_right_entry),
                path: right_file,
            };

            let maybe_compare = api::local::compare::get_cached_compare(
                &repo,
                "no_id",
                new_compare_entry_1.clone(),
                new_compare_entry_2.clone(),
            )?;
            assert!(maybe_compare.is_none());

            // Create the compare and add to the cache to ensure proper update
            let result = api::local::compare::compare_files(
                CompareStrategy::Join,
                &repo,
                Some("a_compare_id"),
                new_compare_entry_1,
                new_compare_entry_2,
                vec![
                    String::from("height"),
                    String::from("weight"),
                    String::from("gender"),
                ],
                vec![String::from("target"), String::from("other_target")],
                None,
            )?;

            if let CompareResult::Tabular(new_compare) = result {
                // Should be updated values
                assert_eq!(new_compare.derived["left_only"].size.height, 0);
                assert_eq!(new_compare.derived["right_only"].size.height, 6);
                assert_eq!(new_compare.derived["match"].size.height, 6);
                assert_eq!(new_compare.derived["diff"].size.height, 0);
            } else {
                assert_eq!(true, false, "Wrong result type for input files")
            }

            Ok(())
        })
    }
}
