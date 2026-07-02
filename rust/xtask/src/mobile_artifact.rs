use crate::common::{command_exists, print_info, print_success, print_warning};
use clap::{Args, Subcommand, ValueEnum};
use color_eyre::{
    eyre::{eyre, Context, ContextCompat},
    Result,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use zip::ZipArchive;

const WORKFLOW_FILE: &str = "mobile-artifacts.yml";
const GITHUB_REPO: &str = "bitcoinppl/cove";
const SCHEMA_VERSION: u32 = 1;
const DEBUG_PROFILE: &str = "debug";
const ANDROID_ARTIFACT_NAME: &str = "cove-android-dev-debug";
const IOS_CORE_ARTIFACT_NAME: &str = "cove-ios-core-debug-device";
const ARTIFACT_ROOT: &str = "_artifacts/mobile";
const ANDROID_APK_PATH: &str = "android/app-dev-debug.apk";
const ANDROID_OUTPUT_METADATA_PATH: &str = "android/output-metadata.json";
const MANIFEST_PATH: &str = "manifest.json";
const ANDROID_PACKAGE_NAME: &str = "org.bitcoinppl.cove.dev";
const ANDROID_ACTIVITY_NAME: &str = "org.bitcoinppl.cove.MainActivity";
const IOS_XCFRAMEWORK_PATH: &str = "ios/CoveCore/Sources/cove_core_ffi.xcframework";
const IOS_GENERATED_PATH: &str = "ios/CoveCore/Sources/CoveCore/generated";
const IOS_TARGET_DIRS: &[&str] = &[IOS_XCFRAMEWORK_PATH, IOS_GENERATED_PATH];
const DIRTY_WARNING_PATHS: &[&str] = &[
    "rust",
    "rust/bindings",
    "android",
    "android/app/src/main/java/org/bitcoinppl/cove_core",
    "ios",
    "ios/CoveCore/Sources/CoveCore/generated",
];

#[derive(Subcommand)]
pub enum MobileArtifactCommand {
    /// Trigger the mobile artifact workflow for a pushed ref
    Trigger(TriggerArgs),

    /// List completed matching mobile artifact runs and locally valid artifacts
    List(ListArgs),

    /// Download, validate, install, and launch an Android artifact
    #[command(name = "install-android")]
    InstallAndroid(InstallAndroidArgs),

    /// Download, validate, and replace local iOS CoveCore generated inputs
    #[command(name = "fetch-ios-core")]
    FetchIosCore(FetchIosCoreArgs),

    /// Clean local mobile artifact cache entries
    Clean(CleanArgs),
}

#[derive(Args)]
pub struct TriggerArgs {
    #[arg(long, value_enum)]
    platform: TriggerPlatform,

    #[arg(long = "ref")]
    git_ref: String,
}

#[derive(Args)]
pub struct ListArgs {
    #[arg(long = "ref")]
    git_ref: Option<String>,
}

#[derive(Args)]
pub struct InstallAndroidArgs {
    #[arg(long = "ref")]
    git_ref: Option<String>,

    #[arg(long)]
    run_id: Option<String>,

    #[arg(long)]
    allow_mismatch: bool,

    #[arg(long)]
    reset: bool,
}

#[derive(Args)]
pub struct FetchIosCoreArgs {
    #[arg(long = "ref")]
    git_ref: Option<String>,

    #[arg(long)]
    run_id: Option<String>,

    #[arg(long)]
    allow_mismatch: bool,
}

#[derive(Args)]
pub struct CleanArgs {
    #[arg(long, default_value_t = 7)]
    older_than_days: u64,

    #[arg(long)]
    include_derived_data: bool,

    #[arg(long)]
    include_rust_targets: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum TriggerPlatform {
    Android,
    #[value(name = "ios-core")]
    IosCore,
    Both,
}

impl TriggerPlatform {
    fn workflow_value(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::IosCore => "ios-core",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum ArtifactPlatform {
    Android,
    IosCore,
}

impl ArtifactPlatform {
    fn artifact_name(self) -> &'static str {
        match self {
            Self::Android => ANDROID_ARTIFACT_NAME,
            Self::IosCore => IOS_CORE_ARTIFACT_NAME,
        }
    }

    fn display(self) -> &'static str {
        match self {
            Self::Android => "android",
            Self::IosCore => "ios-core",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct ArtifactManifest {
    schema_version: u32,
    platform: ArtifactPlatform,
    profile: String,
    git_ref: String,
    git_sha: String,
    workflow_run_id: String,
    created_at: String,
    artifact_name: String,
    rustc_version: String,
    runner_os: String,
    #[serde(default)]
    xcodebuild_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WorkflowRun {
    #[serde(rename = "databaseId")]
    database_id: u64,
    #[serde(rename = "headBranch")]
    head_branch: Option<String>,
    #[serde(rename = "headSha")]
    head_sha: String,
    #[serde(rename = "createdAt")]
    created_at: String,
    status: String,
    conclusion: String,
    url: String,
}

#[derive(Debug, Deserialize)]
struct ArtifactsResponse {
    artifacts: Vec<GitHubArtifact>,
}

#[derive(Debug, Deserialize)]
struct GitHubArtifact {
    id: u64,
    name: String,
    expired: bool,
}

#[derive(Debug)]
struct SelectedArtifact {
    run_id: String,
    artifact_dir: PathBuf,
    manifest: ArtifactManifest,
}

pub fn run(command: MobileArtifactCommand, _verbose: bool) -> Result<()> {
    match command {
        MobileArtifactCommand::Trigger(args) => trigger(args),
        MobileArtifactCommand::List(args) => list(args),
        MobileArtifactCommand::InstallAndroid(args) => install_android(args),
        MobileArtifactCommand::FetchIosCore(args) => fetch_ios_core(args),
        MobileArtifactCommand::Clean(args) => clean(args),
    }
}

fn trigger(args: TriggerArgs) -> Result<()> {
    ensure_command("gh")?;

    let status = Command::new("gh")
        .args([
            "workflow",
            "run",
            WORKFLOW_FILE,
            "--ref",
            &args.git_ref,
            "--repo",
            GITHUB_REPO,
            "-f",
            &format!("platform={}", args.platform.workflow_value()),
            "-f",
            "profile=debug",
        ])
        .status()
        .wrap_err("Failed to trigger mobile artifacts workflow through gh")?;

    if !status.success() {
        color_eyre::eyre::bail!("gh workflow run failed");
    }

    print_success(&format!(
        "Triggered {WORKFLOW_FILE} for ref {} platform {}",
        args.git_ref,
        args.platform.workflow_value()
    ));

    Ok(())
}

fn list(args: ListArgs) -> Result<()> {
    let git_ref = args.git_ref.map(Ok).unwrap_or_else(current_branch)?;
    let repo_root = repo_root()?;
    let runs = successful_runs(&git_ref)?;

    if runs.is_empty() {
        print_warning(&format!("No completed successful {WORKFLOW_FILE} runs found for {git_ref}"));
        return Ok(());
    }

    for run in runs {
        println!(
            "run={} ref={} sha={} created={} url={}",
            run.database_id,
            run.head_branch.as_deref().unwrap_or("<unknown>"),
            run.head_sha,
            run.created_at,
            run.url
        );

        for platform in [ArtifactPlatform::Android, ArtifactPlatform::IosCore] {
            let result = download_validate_and_stage(
                &repo_root,
                &run.database_id.to_string(),
                platform,
                Some(&git_ref),
                true,
            );

            match result {
                Ok(artifact) => println!(
                    "  {} artifact={} manifest_sha={} staged={}",
                    platform.display(),
                    artifact.manifest.artifact_name,
                    artifact.manifest.git_sha,
                    artifact.artifact_dir.display()
                ),
                Err(error) => println!("  {} unavailable: {error:#}", platform.display()),
            }
        }
    }

    Ok(())
}

fn install_android(args: InstallAndroidArgs) -> Result<()> {
    ensure_command("adb")?;
    warn_on_dirty_artifact_inputs()?;

    let repo_root = repo_root()?;
    let artifact = select_artifact(
        &repo_root,
        ArtifactPlatform::Android,
        args.git_ref.as_deref(),
        args.run_id.as_deref(),
        args.allow_mismatch,
    )?;
    let apk_path = artifact.artifact_dir.join(ANDROID_APK_PATH);

    validate_android_apk_identity(&apk_path)?;
    install_apk(&apk_path, args.reset)?;

    println!(
        "Installed Android artifact {} from run {}",
        artifact.manifest.artifact_name, artifact.run_id
    );

    Ok(())
}

fn fetch_ios_core(args: FetchIosCoreArgs) -> Result<()> {
    warn_on_dirty_artifact_inputs()?;
    ensure_clean_ios_targets()?;

    let repo_root = repo_root()?;
    let artifact = select_artifact(
        &repo_root,
        ArtifactPlatform::IosCore,
        args.git_ref.as_deref(),
        args.run_id.as_deref(),
        args.allow_mismatch,
    )?;

    replace_ios_core_inputs(&repo_root, &artifact.artifact_dir)?;
    println!(
        "Fetched iOS CoveCore artifact {} from run {}",
        artifact.manifest.artifact_name, artifact.run_id
    );

    Ok(())
}

fn clean(args: CleanArgs) -> Result<()> {
    let repo_root = repo_root()?;
    let cutoff = cutoff_time(args.older_than_days)?;
    let artifact_root = repo_root.join(ARTIFACT_ROOT);

    remove_old_children(&artifact_root.join("tmp"), cutoff, true)?;
    remove_old_children(&artifact_root.join("runs"), cutoff, false)?;

    if args.include_derived_data {
        let derived_data = derived_data_root()?;
        remove_allowlisted_children(&derived_data, cutoff, |name| name.starts_with("Cove-"))?;
    }

    if args.include_rust_targets {
        remove_allowlisted_path(&repo_root.join("rust/target"), cutoff)?;
    }

    print_success("Mobile artifact cleanup completed");
    Ok(())
}

fn select_artifact(
    repo_root: &Path,
    platform: ArtifactPlatform,
    git_ref: Option<&str>,
    run_id: Option<&str>,
    allow_mismatch: bool,
) -> Result<SelectedArtifact> {
    if let Some(run_id) = run_id {
        return download_validate_and_stage(repo_root, run_id, platform, git_ref, allow_mismatch);
    }

    let git_ref = match git_ref {
        Some(git_ref) => git_ref.to_string(),
        None => current_branch()?,
    };

    for run in successful_runs(&git_ref)? {
        let run_id = run.database_id.to_string();
        match download_validate_and_stage(
            repo_root,
            &run_id,
            platform,
            Some(&git_ref),
            allow_mismatch,
        ) {
            Ok(artifact) => return Ok(artifact),
            Err(error) => {
                print_warning(&format!(
                    "Skipping run {run_id} for {}: {error:#}",
                    platform.display()
                ));
            }
        }
    }

    color_eyre::eyre::bail!("No valid {} artifact found for ref {git_ref}", platform.display());
}

fn download_validate_and_stage(
    repo_root: &Path,
    run_id: &str,
    platform: ArtifactPlatform,
    expected_ref: Option<&str>,
    allow_mismatch: bool,
) -> Result<SelectedArtifact> {
    let artifact_name = platform.artifact_name();
    let tmp_dir = repo_root.join(ARTIFACT_ROOT).join("tmp").join(unique_tmp_name(run_id));
    let artifact_tmp = tmp_dir.join(artifact_name);
    let archive_path = tmp_dir.join(format!("{artifact_name}.zip"));
    let staged_dir = repo_root.join(ARTIFACT_ROOT).join("runs").join(run_id).join(artifact_name);

    recreate_dir(&tmp_dir)?;
    gh_download(run_id, artifact_name, &archive_path)?;
    validate_and_extract_zip(&archive_path, &artifact_tmp)?;
    validate_tree_safety(&artifact_tmp)?;

    let manifest = read_manifest(&artifact_tmp)?;
    validate_manifest(&manifest, platform, artifact_name, expected_ref, run_id)?;
    validate_expected_paths(&artifact_tmp, platform)?;
    validate_head_match(&manifest, allow_mismatch)?;

    if staged_dir.exists() {
        fs::remove_dir_all(&staged_dir).wrap_err_with(|| {
            format!("Failed to remove old staged artifact {}", staged_dir.display())
        })?;
    }

    let staged_parent =
        staged_dir.parent().ok_or_else(|| eyre!("staged artifact path has no parent"))?;
    fs::create_dir_all(staged_parent)
        .wrap_err_with(|| format!("Failed to create {}", staged_parent.display()))?;
    fs::rename(&artifact_tmp, &staged_dir)
        .or_else(|_| copy_dir_replace(&artifact_tmp, &staged_dir))
        .wrap_err_with(|| format!("Failed to stage artifact at {}", staged_dir.display()))?;
    let _ = fs::remove_dir_all(&tmp_dir);

    Ok(SelectedArtifact { run_id: run_id.to_string(), artifact_dir: staged_dir, manifest })
}

fn gh_download(run_id: &str, artifact_name: &str, dest: &Path) -> Result<()> {
    ensure_command("gh")?;

    let artifact = github_artifact(run_id, artifact_name)?;
    let output =
        File::create(dest).wrap_err_with(|| format!("Failed to create {}", dest.display()))?;
    let status = Command::new("gh")
        .args([
            "api",
            "-H",
            "Accept: application/vnd.github+json",
            &format!("/repos/{GITHUB_REPO}/actions/artifacts/{}/zip", artifact.id),
        ])
        .stdout(Stdio::from(output))
        .status()
        .wrap_err_with(|| {
            format!("Failed to download artifact {artifact_name} from run {run_id}")
        })?;

    if !status.success() {
        color_eyre::eyre::bail!(
            "gh artifact download failed for run {run_id} artifact {artifact_name}"
        );
    }

    Ok(())
}

fn github_artifact(run_id: &str, artifact_name: &str) -> Result<GitHubArtifact> {
    let output = Command::new("gh")
        .args([
            "api",
            "-H",
            "Accept: application/vnd.github+json",
            &format!("/repos/{GITHUB_REPO}/actions/runs/{run_id}/artifacts"),
        ])
        .output()
        .wrap_err_with(|| format!("Failed to list artifacts for run {run_id}"))?;

    if !output.status.success() {
        color_eyre::eyre::bail!("gh api artifact listing failed for run {run_id}");
    }

    let response: ArtifactsResponse =
        serde_json::from_slice(&output.stdout).wrap_err("Failed to parse GitHub artifact list")?;
    response
        .artifacts
        .into_iter()
        .find(|artifact| artifact.name == artifact_name && !artifact.expired)
        .ok_or_else(|| eyre!("Run {run_id} does not have non-expired artifact {artifact_name}"))
}

fn successful_runs(git_ref: &str) -> Result<Vec<WorkflowRun>> {
    ensure_command("gh")?;

    let output = Command::new("gh")
        .args([
            "run",
            "list",
            "--repo",
            GITHUB_REPO,
            "--workflow",
            WORKFLOW_FILE,
            "--branch",
            git_ref,
            "--status",
            "success",
            "--limit",
            "20",
            "--json",
            "databaseId,headBranch,headSha,createdAt,status,conclusion,url",
        ])
        .output()
        .wrap_err("Failed to list mobile artifact workflow runs through gh")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("gh run list failed");
    }

    let runs: Vec<WorkflowRun> = serde_json::from_slice(&output.stdout)
        .wrap_err("Failed to parse gh run list JSON for mobile artifacts")?;

    Ok(runs
        .into_iter()
        .filter(|run| run.status == "completed" && run.conclusion == "success")
        .collect())
}

fn read_manifest(artifact_dir: &Path) -> Result<ArtifactManifest> {
    let manifest_path = artifact_dir.join(MANIFEST_PATH);
    let manifest = fs::read_to_string(&manifest_path)
        .wrap_err_with(|| format!("Failed to read {}", manifest_path.display()))?;

    serde_json::from_str(&manifest)
        .wrap_err_with(|| format!("Failed to parse {}", manifest_path.display()))
}

fn validate_manifest(
    manifest: &ArtifactManifest,
    platform: ArtifactPlatform,
    artifact_name: &str,
    expected_ref: Option<&str>,
    run_id: &str,
) -> Result<()> {
    if manifest.schema_version != SCHEMA_VERSION {
        color_eyre::eyre::bail!(
            "Unknown mobile artifact manifest schema_version {}",
            manifest.schema_version
        );
    }

    if manifest.platform != platform {
        color_eyre::eyre::bail!(
            "Manifest platform {} does not match requested {}",
            manifest.platform.display(),
            platform.display()
        );
    }

    if manifest.profile != DEBUG_PROFILE {
        color_eyre::eyre::bail!("Refusing non-debug mobile artifact profile {}", manifest.profile);
    }

    if manifest.git_ref.trim().is_empty() {
        color_eyre::eyre::bail!("Manifest git_ref is empty");
    }

    if manifest.artifact_name != artifact_name {
        color_eyre::eyre::bail!(
            "Manifest artifact_name {} does not match expected {artifact_name}",
            manifest.artifact_name
        );
    }

    if !manifest.workflow_run_id.chars().all(|c| c.is_ascii_digit()) {
        color_eyre::eyre::bail!(
            "Manifest workflow_run_id is not numeric: {}",
            manifest.workflow_run_id
        );
    }

    if manifest.workflow_run_id != run_id {
        color_eyre::eyre::bail!(
            "Manifest workflow_run_id {} does not match selected run {run_id}",
            manifest.workflow_run_id
        );
    }

    if expected_ref.is_some_and(|expected_ref| manifest.git_ref != expected_ref) {
        let expected_ref = expected_ref.unwrap();
        color_eyre::eyre::bail!(
            "Manifest git_ref {} does not match requested ref {expected_ref}",
            manifest.git_ref
        );
    }

    validate_sha(&manifest.git_sha)?;
    validate_created_at(&manifest.created_at)?;
    validate_required_text("rustc_version", &manifest.rustc_version)?;
    validate_required_text("runner_os", &manifest.runner_os)?;
    if platform == ArtifactPlatform::IosCore {
        validate_required_text(
            "xcodebuild_version",
            manifest.xcodebuild_version.as_deref().unwrap_or_default(),
        )?;
    }

    Ok(())
}

fn validate_sha(sha: &str) -> Result<()> {
    if sha.len() == 40 && sha.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(());
    }

    color_eyre::eyre::bail!("Manifest git_sha is not a 40-character SHA: {sha}");
}

fn validate_required_text(field: &str, value: &str) -> Result<()> {
    if value.trim().is_empty() {
        color_eyre::eyre::bail!("Manifest {field} is empty");
    }

    Ok(())
}

fn validate_created_at(value: &str) -> Result<()> {
    validate_required_text("created_at", value)?;

    if value.contains('T') && value.ends_with('Z') {
        return Ok(());
    }

    color_eyre::eyre::bail!("Manifest created_at is not an expected UTC timestamp: {value}");
}

fn validate_head_match(manifest: &ArtifactManifest, allow_mismatch: bool) -> Result<()> {
    let head = current_head_sha()?;
    let comparison = head_mismatch(&head, &manifest.git_sha);

    if comparison == HeadComparison::Matches || allow_mismatch {
        if comparison == HeadComparison::Mismatches {
            print_warning(&format!(
                "Using artifact for git_sha {} while workspace HEAD is {}",
                manifest.git_sha, head
            ));
        }

        return Ok(());
    }

    color_eyre::eyre::bail!(
        "Workspace HEAD {head} differs from artifact git_sha {}; pass --allow-mismatch to use it deliberately",
        manifest.git_sha
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HeadComparison {
    Matches,
    Mismatches,
}

fn head_mismatch(head: &str, manifest_sha: &str) -> HeadComparison {
    if head == manifest_sha {
        HeadComparison::Matches
    } else {
        HeadComparison::Mismatches
    }
}

fn validate_expected_paths(artifact_dir: &Path, platform: ArtifactPlatform) -> Result<()> {
    let expected = match platform {
        ArtifactPlatform::Android => {
            vec![MANIFEST_PATH, ANDROID_APK_PATH, ANDROID_OUTPUT_METADATA_PATH]
        }
        ArtifactPlatform::IosCore => vec![MANIFEST_PATH, IOS_XCFRAMEWORK_PATH, IOS_GENERATED_PATH],
    };

    for relative in expected {
        let path = artifact_dir.join(relative);
        if !path.exists() {
            color_eyre::eyre::bail!("Artifact is missing expected path {}", path.display());
        }
    }

    Ok(())
}

fn validate_and_extract_zip(zip_path: &Path, dest: &Path) -> Result<()> {
    let zip_file =
        File::open(zip_path).wrap_err_with(|| format!("Failed to open {}", zip_path.display()))?;
    let mut archive = ZipArchive::new(zip_file)
        .wrap_err_with(|| format!("Failed to read artifact ZIP {}", zip_path.display()))?;

    for index in 0..archive.len() {
        let file = archive.by_index(index).wrap_err("Failed to inspect artifact ZIP entry")?;
        let enclosed_name = file
            .enclosed_name()
            .ok_or_else(|| eyre!("Artifact ZIP entry is unsafe: {}", file.name()))?;

        validate_zip_entry_path(&enclosed_name)?;
        reject_zip_unsafe_entry_type(file.name(), file.unix_mode())?;
    }

    recreate_dir(dest)?;

    let zip_file =
        File::open(zip_path).wrap_err_with(|| format!("Failed to open {}", zip_path.display()))?;
    let mut archive = ZipArchive::new(zip_file)
        .wrap_err_with(|| format!("Failed to read artifact ZIP {}", zip_path.display()))?;

    for index in 0..archive.len() {
        let mut file = archive.by_index(index).wrap_err("Failed to extract artifact ZIP entry")?;
        let relative = file
            .enclosed_name()
            .ok_or_else(|| eyre!("Artifact ZIP entry is unsafe: {}", file.name()))?;
        let out_path = dest.join(&relative);

        if file.is_dir() {
            fs::create_dir_all(&out_path)
                .wrap_err_with(|| format!("Failed to create {}", out_path.display()))?;
            continue;
        }

        let parent = out_path.parent().wrap_err("ZIP output path has no parent")?;
        fs::create_dir_all(parent)
            .wrap_err_with(|| format!("Failed to create {}", parent.display()))?;
        let mut output = File::create(&out_path)
            .wrap_err_with(|| format!("Failed to create {}", out_path.display()))?;
        io::copy(&mut file, &mut output)
            .wrap_err_with(|| format!("Failed to extract {}", out_path.display()))?;
        apply_zip_file_permissions(&out_path, file.unix_mode())?;
    }

    Ok(())
}

fn validate_zip_entry_path(path: &Path) -> Result<()> {
    validate_relative_path(path)?;
    validate_artifact_top_level(path)
}

fn reject_zip_unsafe_entry_type(name: &str, unix_mode: Option<u32>) -> Result<()> {
    let Some(unix_mode) = unix_mode else {
        return Ok(());
    };

    let file_type = unix_mode & 0o170000;
    match file_type {
        0o120000 => color_eyre::eyre::bail!("Artifact ZIP entry is a symlink: {name}"),
        0o010000 => color_eyre::eyre::bail!("Artifact ZIP entry is a FIFO: {name}"),
        _ => Ok(()),
    }
}

fn validate_tree_safety(root: &Path) -> Result<()> {
    let canonical_root =
        root.canonicalize().wrap_err_with(|| format!("Failed to resolve {}", root.display()))?;

    validate_tree_safety_inner(root, root, &canonical_root)
}

fn validate_tree_safety_inner(root: &Path, current: &Path, canonical_root: &Path) -> Result<()> {
    for entry in fs::read_dir(current)
        .wrap_err_with(|| format!("Failed to read artifact directory {}", current.display()))?
    {
        let entry = entry.wrap_err("Failed to read artifact directory entry")?;
        let path = entry.path();
        let relative = path
            .strip_prefix(root)
            .wrap_err_with(|| format!("Artifact path escaped root: {}", path.display()))?;

        validate_relative_path(relative)?;
        validate_artifact_top_level(relative)?;

        let metadata = fs::symlink_metadata(&path)
            .wrap_err_with(|| format!("Failed to inspect {}", path.display()))?;
        if metadata.file_type().is_symlink() {
            color_eyre::eyre::bail!("Artifact contains symlink {}", path.display());
        }

        reject_hardlink(&path, &metadata)?;

        let canonical = path
            .canonicalize()
            .wrap_err_with(|| format!("Failed to resolve {}", path.display()))?;
        if !canonical.starts_with(canonical_root) {
            color_eyre::eyre::bail!("Artifact path escapes staging root: {}", path.display());
        }

        if metadata.is_dir() {
            validate_tree_safety_inner(root, &path, canonical_root)?;
        }
    }

    Ok(())
}

fn validate_relative_path(path: &Path) -> Result<()> {
    if path.is_absolute() {
        color_eyre::eyre::bail!("Artifact path is absolute: {}", path.display());
    }

    for component in path.components() {
        if matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_)) {
            color_eyre::eyre::bail!("Artifact path is unsafe: {}", path.display());
        }

        if matches!(&component, Component::Normal(value) if value.to_string_lossy().contains('\\'))
        {
            color_eyre::eyre::bail!(
                "Artifact path contains a Windows separator: {}",
                path.display()
            );
        }
    }

    Ok(())
}

fn validate_artifact_top_level(path: &Path) -> Result<()> {
    let Some(top_level) = path.components().next().and_then(|component| match component {
        Component::Normal(value) => Some(value),
        _ => None,
    }) else {
        return Ok(());
    };

    if matches!(top_level.to_str(), Some("manifest.json" | "android" | "ios")) {
        return Ok(());
    }

    color_eyre::eyre::bail!("Artifact contains unexpected top-level path {}", path.display());
}

#[cfg(unix)]
fn reject_hardlink(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    use std::os::unix::fs::MetadataExt;

    if metadata.is_file() && metadata.nlink() > 1 {
        color_eyre::eyre::bail!("Artifact contains hardlink {}", path.display());
    }

    Ok(())
}

#[cfg(not(unix))]
fn reject_hardlink(_path: &Path, _metadata: &fs::Metadata) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn apply_zip_file_permissions(path: &Path, unix_mode: Option<u32>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let Some(unix_mode) = unix_mode else {
        return Ok(());
    };

    let permission_bits = unix_mode & 0o777;
    if permission_bits == 0 {
        return Ok(());
    }

    let permissions = fs::Permissions::from_mode(permission_bits);
    fs::set_permissions(path, permissions)
        .wrap_err_with(|| format!("Failed to set permissions on {}", path.display()))
}

#[cfg(not(unix))]
fn apply_zip_file_permissions(_path: &Path, _unix_mode: Option<u32>) -> Result<()> {
    Ok(())
}

fn validate_android_apk_identity(apk_path: &Path) -> Result<()> {
    let package = read_apk_package_name(apk_path)?;

    if package == ANDROID_PACKAGE_NAME {
        return Ok(());
    }

    color_eyre::eyre::bail!(
        "APK package name {package} does not match expected {ANDROID_PACKAGE_NAME}"
    );
}

fn read_apk_package_name(apk_path: &Path) -> Result<String> {
    if let Some(aapt) = find_android_tool("aapt") {
        let output = Command::new(aapt)
            .args(["dump", "badging"])
            .arg(apk_path)
            .output()
            .wrap_err("Failed to inspect APK with aapt")?;

        if output.status.success() {
            let stdout = String::from_utf8(output.stdout)
                .wrap_err("aapt output for APK identity was not valid UTF-8")?;

            return parse_aapt_package_name(&stdout)
                .ok_or_else(|| eyre!("aapt output did not include package name"));
        }
    }

    if let Some(apkanalyzer) = find_android_tool("apkanalyzer") {
        let output = Command::new(apkanalyzer)
            .args(["manifest", "application-id"])
            .arg(apk_path)
            .output()
            .wrap_err("Failed to inspect APK with apkanalyzer")?;

        if output.status.success() {
            let package = String::from_utf8(output.stdout)
                .wrap_err("apkanalyzer output for APK identity was not valid UTF-8")?
                .trim()
                .to_string();

            if !package.is_empty() {
                return Ok(package);
            }
        }
    }

    color_eyre::eyre::bail!(
        "Unable to inspect APK identity; install Android SDK build-tools with aapt or apkanalyzer"
    );
}

fn parse_aapt_package_name(output: &str) -> Option<String> {
    let line = output.lines().find(|line| line.starts_with("package: "))?;
    let name_start = line.find("name='")? + "name='".len();
    let rest = &line[name_start..];
    let name_end = rest.find('\'')?;

    Some(rest[..name_end].to_string())
}

fn find_android_tool(name: &str) -> Option<PathBuf> {
    if command_exists(name) {
        return Some(PathBuf::from(name));
    }

    for env_name in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        let Some(root) = std::env::var_os(env_name) else {
            continue;
        };
        let build_tools = PathBuf::from(root).join("build-tools");
        let Ok(entries) = fs::read_dir(build_tools) else {
            continue;
        };

        let mut candidates = entries
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path().join(name))
            .filter(|path| path.exists())
            .collect::<Vec<_>>();
        candidates.sort();

        if let Some(candidate) = candidates.pop() {
            return Some(candidate);
        }
    }

    None
}

fn install_apk(apk_path: &Path, reset: bool) -> Result<()> {
    print_info(&format!("Installing APK {}", apk_path.display()));
    let output = Command::new("adb")
        .args(["install", "-r"])
        .arg(apk_path)
        .output()
        .wrap_err("Failed to run adb install")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("INSTALL_FAILED_UPDATE_INCOMPATIBLE") {
            color_eyre::eyre::bail!(
                "adb install failed with INSTALL_FAILED_UPDATE_INCOMPATIBLE. Uninstall the existing dev app or use an approved shared dev debug key before retrying.\n{stderr}"
            );
        }

        color_eyre::eyre::bail!("adb install failed: {stderr}");
    }

    if reset {
        print_info("Clearing Android dev app data");
        run_status(Command::new("adb").args(["shell", "pm", "clear", ANDROID_PACKAGE_NAME]))?;
    }

    let component = format!("{ANDROID_PACKAGE_NAME}/{ANDROID_ACTIVITY_NAME}");
    print_info("Launching Android dev app");
    run_status(Command::new("adb").args(["shell", "am", "start", "-n", &component]))?;

    print_success("Android artifact installed and launched");
    Ok(())
}

fn run_status(command: &mut Command) -> Result<()> {
    let status = command.status().wrap_err("Failed to run command")?;
    if !status.success() {
        color_eyre::eyre::bail!("command failed with status {status}");
    }

    Ok(())
}

fn replace_ios_core_inputs(repo_root: &Path, artifact_dir: &Path) -> Result<()> {
    let backup_root = repo_root.join(ARTIFACT_ROOT).join("tmp").join(unique_tmp_name("ios-backup"));
    recreate_dir(&backup_root)?;

    let mut replacement = IosReplacement::default();
    let result =
        replace_ios_core_inputs_inner(repo_root, artifact_dir, &backup_root, &mut replacement);
    if let Err(error) = result {
        rollback_ios_core_inputs(repo_root, &backup_root, &replacement)?;
        return Err(error).wrap_err("Rolled back iOS CoveCore artifact fetch after copy failure");
    }

    let _ = fs::remove_dir_all(&backup_root);
    Ok(())
}

fn replace_ios_core_inputs_inner(
    repo_root: &Path,
    artifact_dir: &Path,
    backup_root: &Path,
    replacement: &mut IosReplacement,
) -> Result<()> {
    for relative in IOS_TARGET_DIRS {
        let target = repo_root.join(relative);
        let backup = backup_root.join(relative);
        if target.exists() {
            move_path(&target, &backup)?;
            replacement.moved_targets.push((*relative).to_string());
        }
    }

    for relative in IOS_TARGET_DIRS {
        let source = artifact_dir.join(relative);
        let target = repo_root.join(relative);
        copy_path(&source, &target)?;
        replacement.copied_targets.push((*relative).to_string());
    }

    Ok(())
}

#[derive(Default)]
struct IosReplacement {
    moved_targets: Vec<String>,
    copied_targets: Vec<String>,
}

fn rollback_ios_core_inputs(
    repo_root: &Path,
    backup_root: &Path,
    replacement: &IosReplacement,
) -> Result<()> {
    for relative in &replacement.copied_targets {
        let target = repo_root.join(relative);
        if target.exists() {
            remove_path(&target)?;
        }
    }

    for relative in &replacement.moved_targets {
        let backup = backup_root.join(relative);
        let target = repo_root.join(relative);
        if backup.exists() {
            move_path(&backup, &target)?;
        }
    }

    Ok(())
}

fn warn_on_dirty_artifact_inputs() -> Result<()> {
    let dirty = git_status_paths(DIRTY_WARNING_PATHS)?;
    if dirty.trim().is_empty() {
        return Ok(());
    }

    print_warning("Workspace has local changes that may not be reflected in pushed artifacts:");
    for line in dirty.lines() {
        println!("  {line}");
    }

    Ok(())
}

fn ensure_clean_ios_targets() -> Result<()> {
    let dirty = git_status_paths(IOS_TARGET_DIRS)?;
    if dirty.trim().is_empty() {
        return Ok(());
    }

    color_eyre::eyre::bail!(
        "Refusing to fetch iOS artifact because generated CoveCore target paths are dirty:\n{dirty}"
    );
}

fn git_status_paths(paths: &[&str]) -> Result<String> {
    let root = repo_root()?;

    git_status_paths_at(&root, paths)
}

fn git_status_paths_at(root: &Path, paths: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(root)
        .args(["status", "--porcelain", "--untracked-files=all", "--"])
        .args(paths)
        .output()
        .wrap_err("Failed to inspect git status")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("git status failed");
    }

    String::from_utf8(output.stdout).wrap_err("git status output was not valid UTF-8")
}

fn current_head_sha() -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .wrap_err("Failed to read current git HEAD")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("git rev-parse HEAD failed");
    }

    Ok(String::from_utf8(output.stdout)
        .wrap_err("git HEAD output was not valid UTF-8")?
        .trim()
        .to_string())
}

fn current_branch() -> Result<String> {
    let output = Command::new("git")
        .args(["symbolic-ref", "--quiet", "--short", "HEAD"])
        .output()
        .wrap_err("Failed to read current git branch")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("--ref is required when current workspace is detached");
    }

    let branch = String::from_utf8(output.stdout)
        .wrap_err("Current git branch was not valid UTF-8")?
        .trim()
        .to_string();

    if branch.is_empty() {
        color_eyre::eyre::bail!("--ref is required when current branch is empty");
    }

    Ok(branch)
}

fn repo_root() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .wrap_err("Failed to locate git repository root")?;

    if !output.status.success() {
        color_eyre::eyre::bail!("git rev-parse --show-toplevel failed");
    }

    let path = String::from_utf8(output.stdout)
        .wrap_err("git repository root was not valid UTF-8")?
        .trim()
        .to_string();

    Ok(PathBuf::from(path))
}

fn ensure_command(command: &str) -> Result<()> {
    if command_exists(command) {
        return Ok(());
    }

    color_eyre::eyre::bail!("{command} is required for mobile artifact commands");
}

fn unique_tmp_name(prefix: &str) -> String {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();

    format!("{prefix}-{pid}-{nanos}")
}

fn recreate_dir(path: &Path) -> Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
            .wrap_err_with(|| format!("Failed to remove {}", path.display()))?;
    }

    fs::create_dir_all(path).wrap_err_with(|| format!("Failed to create {}", path.display()))
}

fn copy_dir_replace(source: &Path, target: &Path) -> Result<()> {
    if target.exists() {
        fs::remove_dir_all(target)
            .wrap_err_with(|| format!("Failed to remove {}", target.display()))?;
    }

    copy_path(source, target)
}

fn copy_path(source: &Path, target: &Path) -> Result<()> {
    if source.is_dir() {
        fs::create_dir_all(target)
            .wrap_err_with(|| format!("Failed to create {}", target.display()))?;

        for entry in
            fs::read_dir(source).wrap_err_with(|| format!("Failed to read {}", source.display()))?
        {
            let entry = entry.wrap_err("Failed to read directory entry")?;
            copy_path(&entry.path(), &target.join(entry.file_name()))?;
        }

        return Ok(());
    }

    let parent = target.parent().wrap_err("copy target has no parent")?;
    fs::create_dir_all(parent)
        .wrap_err_with(|| format!("Failed to create {}", parent.display()))?;
    fs::copy(source, target)
        .wrap_err_with(|| format!("Failed to copy {} to {}", source.display(), target.display()))?;

    Ok(())
}

fn move_path(source: &Path, target: &Path) -> Result<()> {
    let parent = target.parent().wrap_err("move target has no parent")?;
    fs::create_dir_all(parent)
        .wrap_err_with(|| format!("Failed to create {}", parent.display()))?;
    fs::rename(source, target).or_else(|_| {
        copy_path(source, target)?;
        remove_path(source)
    })
}

fn remove_path(path: &Path) -> Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
            .wrap_err_with(|| format!("Failed to remove {}", path.display()))?;
    } else if path.exists() {
        fs::remove_file(path).wrap_err_with(|| format!("Failed to remove {}", path.display()))?;
    }

    Ok(())
}

fn cutoff_time(days: u64) -> Result<SystemTime> {
    let duration = Duration::from_secs(days.saturating_mul(24 * 60 * 60));
    SystemTime::now().checked_sub(duration).ok_or_else(|| eyre!("Invalid cleanup cutoff"))
}

fn remove_old_children(
    path: &Path,
    cutoff: SystemTime,
    remove_root_when_empty: bool,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    for entry in
        fs::read_dir(path).wrap_err_with(|| format!("Failed to read {}", path.display()))?
    {
        let entry = entry.wrap_err("Failed to read cleanup entry")?;
        remove_allowlisted_path_if_old(&entry.path(), cutoff)?;
    }

    if remove_root_when_empty && fs::read_dir(path)?.next().is_none() {
        fs::remove_dir(path).wrap_err_with(|| format!("Failed to remove {}", path.display()))?;
    }

    Ok(())
}

fn remove_allowlisted_children<F>(path: &Path, cutoff: SystemTime, allow: F) -> Result<()>
where
    F: Fn(&str) -> bool,
{
    if !path.exists() {
        return Ok(());
    }

    for entry in
        fs::read_dir(path).wrap_err_with(|| format!("Failed to read {}", path.display()))?
    {
        let entry = entry.wrap_err("Failed to read cleanup entry")?;
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();
        if allow(&name) {
            remove_allowlisted_path_if_old(&entry.path(), cutoff)?;
        }
    }

    Ok(())
}

fn remove_allowlisted_path(path: &Path, cutoff: SystemTime) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    remove_allowlisted_path_if_old(path, cutoff)
}

fn remove_allowlisted_path_if_old(path: &Path, cutoff: SystemTime) -> Result<()> {
    let metadata =
        fs::metadata(path).wrap_err_with(|| format!("Failed to inspect {}", path.display()))?;
    let modified = metadata
        .modified()
        .wrap_err_with(|| format!("Failed to read modified time for {}", path.display()))?;

    if modified > cutoff {
        return Ok(());
    }

    print_info(&format!("Removing {}", path.display()));
    remove_path(path)
}

fn derived_data_root() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").wrap_err("HOME is required for DerivedData cleanup")?;

    Ok(PathBuf::from(home).join("Library/Developer/Xcode/DerivedData"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest(platform: ArtifactPlatform) -> ArtifactManifest {
        ArtifactManifest {
            schema_version: SCHEMA_VERSION,
            platform,
            profile: DEBUG_PROFILE.to_string(),
            git_ref: "wk2".to_string(),
            git_sha: "0123456789abcdef0123456789abcdef01234567".to_string(),
            workflow_run_id: "123".to_string(),
            created_at: "2026-07-01T00:00:00Z".to_string(),
            artifact_name: platform.artifact_name().to_string(),
            rustc_version: "rustc 1.0.0".to_string(),
            runner_os: "Linux".to_string(),
            xcodebuild_version: None,
        }
    }

    #[test]
    fn validates_matching_manifest() {
        let manifest = sample_manifest(ArtifactPlatform::Android);

        validate_manifest(
            &manifest,
            ArtifactPlatform::Android,
            ANDROID_ARTIFACT_NAME,
            Some("wk2"),
            "123",
        )
        .unwrap();
    }

    #[test]
    fn rejects_unknown_schema() {
        let mut manifest = sample_manifest(ArtifactPlatform::Android);
        manifest.schema_version = 2;

        assert!(validate_manifest(
            &manifest,
            ArtifactPlatform::Android,
            ANDROID_ARTIFACT_NAME,
            Some("wk2"),
            "123",
        )
        .is_err());
    }

    #[test]
    fn rejects_non_debug_profile() {
        let mut manifest = sample_manifest(ArtifactPlatform::Android);
        manifest.profile = "release".to_string();

        assert!(validate_manifest(
            &manifest,
            ArtifactPlatform::Android,
            ANDROID_ARTIFACT_NAME,
            Some("wk2"),
            "123",
        )
        .is_err());
    }

    #[test]
    fn rejects_platform_mismatch() {
        let manifest = sample_manifest(ArtifactPlatform::Android);

        assert!(validate_manifest(
            &manifest,
            ArtifactPlatform::IosCore,
            IOS_CORE_ARTIFACT_NAME,
            Some("wk2"),
            "123",
        )
        .is_err());
    }

    #[test]
    fn rejects_artifact_name_mismatch() {
        let manifest = sample_manifest(ArtifactPlatform::Android);

        assert!(validate_manifest(
            &manifest,
            ArtifactPlatform::Android,
            IOS_CORE_ARTIFACT_NAME,
            Some("wk2"),
            "123",
        )
        .is_err());
    }

    #[test]
    fn ios_manifest_requires_xcodebuild_version() {
        let manifest = sample_manifest(ArtifactPlatform::IosCore);

        assert!(validate_manifest(
            &manifest,
            ArtifactPlatform::IosCore,
            IOS_CORE_ARTIFACT_NAME,
            Some("wk2"),
            "123",
        )
        .is_err());
    }

    #[test]
    fn ios_manifest_accepts_xcodebuild_version() {
        let mut manifest = sample_manifest(ArtifactPlatform::IosCore);
        manifest.xcodebuild_version = Some("Xcode 26.0\nBuild version 1A1".to_string());

        validate_manifest(
            &manifest,
            ArtifactPlatform::IosCore,
            IOS_CORE_ARTIFACT_NAME,
            Some("wk2"),
            "123",
        )
        .unwrap();
    }

    #[test]
    fn classifies_head_mismatch() {
        assert_eq!(
            head_mismatch(
                "0123456789abcdef0123456789abcdef01234567",
                "0123456789abcdef0123456789abcdef01234567",
            ),
            HeadComparison::Matches
        );
        assert_eq!(head_mismatch("a", "b"), HeadComparison::Mismatches);
    }

    #[test]
    fn rejects_unsafe_relative_paths() {
        assert!(validate_relative_path(Path::new("../manifest.json")).is_err());
        assert!(validate_relative_path(Path::new("android\\app.apk")).is_err());
        assert!(validate_relative_path(Path::new("android/app.apk")).is_ok());
    }

    #[test]
    fn rejects_unsafe_zip_entry_paths() {
        assert!(validate_zip_entry_path(Path::new("../manifest.json")).is_err());
        assert!(validate_zip_entry_path(Path::new("unexpected/file")).is_err());
        assert!(validate_zip_entry_path(Path::new("manifest.json")).is_ok());
    }

    #[test]
    fn rejects_zip_unsafe_entry_types() {
        assert!(reject_zip_unsafe_entry_type("link", Some(0o120777)).is_err());
        assert!(reject_zip_unsafe_entry_type("fifo", Some(0o010777)).is_err());
        assert!(reject_zip_unsafe_entry_type("file", Some(0o100644)).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn extract_zip_preserves_unix_file_permissions() {
        use std::io::Write;
        use std::os::unix::fs::PermissionsExt;
        use zip::write::SimpleFileOptions;

        let root = tempfile::tempdir().unwrap();
        let zip_path = root.path().join("artifact.zip");
        let zip_file = File::create(&zip_path).unwrap();
        let mut zip = zip::ZipWriter::new(zip_file);
        let options = SimpleFileOptions::default().unix_permissions(0o755);

        zip.start_file(ANDROID_APK_PATH, options).unwrap();
        zip.write_all(b"apk").unwrap();
        zip.finish().unwrap();

        let dest = root.path().join("dest");
        validate_and_extract_zip(&zip_path, &dest).unwrap();

        let mode = fs::metadata(dest.join(ANDROID_APK_PATH)).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn git_status_paths_at_uses_repo_root_relative_pathspecs() {
        let root = tempfile::tempdir().unwrap();
        let repo_root = root.path();

        let status =
            Command::new("git").current_dir(repo_root).args(["init", "--quiet"]).status().unwrap();
        assert!(status.success());

        let dirty_path = repo_root.join(IOS_GENERATED_PATH).join("dirty.swift");
        fs::create_dir_all(dirty_path.parent().unwrap()).unwrap();
        fs::write(&dirty_path, "dirty").unwrap();

        let dirty = git_status_paths_at(repo_root, &[IOS_GENERATED_PATH]).unwrap();

        assert!(dirty.contains("ios/CoveCore/Sources/CoveCore/generated/dirty.swift"));
    }

    #[test]
    fn rejects_unexpected_top_level_paths() {
        assert!(validate_artifact_top_level(Path::new("tmp/file")).is_err());
        assert!(validate_artifact_top_level(Path::new("android/app-dev-debug.apk")).is_ok());
        assert!(validate_artifact_top_level(Path::new("ios/CoveCore")).is_ok());
    }

    #[test]
    fn parses_aapt_package_name() {
        let output = "package: name='org.bitcoinppl.cove.dev' versionCode='1'";

        assert_eq!(parse_aapt_package_name(output).as_deref(), Some("org.bitcoinppl.cove.dev"));
    }

    #[test]
    fn validates_sha_shape() {
        assert!(validate_sha("0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(validate_sha("not-a-sha").is_err());
    }

    #[test]
    fn missing_expected_path_is_rejected() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(MANIFEST_PATH), "{}").unwrap();

        assert!(validate_expected_paths(root.path(), ArtifactPlatform::Android).is_err());
    }

    #[test]
    fn expected_android_paths_are_accepted() {
        let root = tempfile::tempdir().unwrap();
        let android = root.path().join("android");
        fs::create_dir_all(&android).unwrap();
        fs::write(root.path().join(MANIFEST_PATH), "{}").unwrap();
        fs::write(root.path().join(ANDROID_APK_PATH), "apk").unwrap();
        fs::write(root.path().join(ANDROID_OUTPUT_METADATA_PATH), "{}").unwrap();

        assert!(validate_expected_paths(root.path(), ArtifactPlatform::Android).is_ok());
    }

    #[test]
    fn tree_safety_rejects_unexpected_top_level_file() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("unexpected.txt"), "no").unwrap();

        assert!(validate_tree_safety(root.path()).is_err());
    }

    #[test]
    fn tree_safety_accepts_expected_layout() {
        let root = tempfile::tempdir().unwrap();
        let android = root.path().join("android");
        fs::create_dir_all(&android).unwrap();
        fs::write(root.path().join(MANIFEST_PATH), "{}").unwrap();
        fs::write(android.join("app-dev-debug.apk"), "apk").unwrap();
        fs::write(android.join("output-metadata.json"), "{}").unwrap();

        assert!(validate_tree_safety(root.path()).is_ok());
    }

    #[test]
    fn copy_path_copies_directories() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let target = root.path().join("target");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/file.swift"), "swift").unwrap();

        copy_path(&source, &target).unwrap();

        assert_eq!(fs::read_to_string(target.join("nested/file.swift")).unwrap(), "swift");
    }

    #[test]
    fn ios_replace_rolls_back_after_copy_failure() {
        let root = tempfile::tempdir().unwrap();
        let repo_root = root.path().join("repo");
        let artifact = root.path().join("artifact");
        let old_xcframework_file = repo_root.join(IOS_XCFRAMEWORK_PATH).join("old.txt");
        let old_generated_file = repo_root.join(IOS_GENERATED_PATH).join("old.swift");
        let new_xcframework_file = artifact.join(IOS_XCFRAMEWORK_PATH).join("new.txt");

        fs::create_dir_all(old_xcframework_file.parent().unwrap()).unwrap();
        fs::create_dir_all(old_generated_file.parent().unwrap()).unwrap();
        fs::create_dir_all(new_xcframework_file.parent().unwrap()).unwrap();
        fs::write(&old_xcframework_file, "old framework").unwrap();
        fs::write(&old_generated_file, "old swift").unwrap();
        fs::write(&new_xcframework_file, "new framework").unwrap();

        assert!(replace_ios_core_inputs(&repo_root, &artifact).is_err());

        assert_eq!(fs::read_to_string(old_xcframework_file).unwrap(), "old framework");
        assert_eq!(fs::read_to_string(old_generated_file).unwrap(), "old swift");
        assert!(!repo_root.join(IOS_XCFRAMEWORK_PATH).join("new.txt").exists());
    }

    #[test]
    fn ios_rollback_leaves_unmoved_original_targets_in_place() {
        let root = tempfile::tempdir().unwrap();
        let repo_root = root.path().join("repo");
        let backup_root = root.path().join("backup");
        let original = repo_root.join(IOS_GENERATED_PATH).join("old.swift");

        fs::create_dir_all(original.parent().unwrap()).unwrap();
        fs::write(&original, "old swift").unwrap();

        rollback_ios_core_inputs(&repo_root, &backup_root, &IosReplacement::default()).unwrap();

        assert_eq!(fs::read_to_string(original).unwrap(), "old swift");
    }

    #[test]
    fn cleanup_skips_new_entries() {
        let root = tempfile::tempdir().unwrap();
        let entry = root.path().join("new");
        fs::create_dir_all(&entry).unwrap();
        let cutoff = SystemTime::now() - Duration::from_secs(60);

        remove_old_children(root.path(), cutoff, false).unwrap();

        assert!(entry.exists());
    }
}
