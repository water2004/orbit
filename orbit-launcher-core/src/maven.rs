use crate::error::LauncherError;

pub(crate) fn artifact_path(
    coordinate: &str,
    classifier_override: Option<&str>,
) -> Result<String, LauncherError> {
    let (coordinate, extension) = coordinate.split_once('@').unwrap_or((coordinate, "jar"));
    let parts: Vec<_> = coordinate.split(':').collect();
    if !(3..=4).contains(&parts.len())
        || parts.iter().any(|part| {
            part.is_empty()
                || part.contains(['/', '\\'])
                || part == &"."
                || part == &".."
                || part.chars().any(char::is_control)
        })
        || extension.is_empty()
        || !extension
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Maven coordinate '{coordinate}' is invalid"
        )));
    }
    let group = parts[0].replace('.', "/");
    let artifact = parts[1];
    let version = parts[2];
    let classifier = classifier_override
        .or_else(|| parts.get(3).copied())
        .map(|classifier| format!("-{classifier}"))
        .unwrap_or_default();
    let filename = format!("{artifact}-{version}{classifier}.{extension}");
    Ok(format!("{group}/{artifact}/{version}/{filename}"))
}

pub(crate) fn artifact_url(
    repository: &str,
    coordinate: &str,
    classifier_override: Option<&str>,
) -> Result<(String, String), LauncherError> {
    let path = artifact_path(coordinate, classifier_override)?;
    let mut repository = url::Url::parse(repository).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "Maven repository '{repository}' is invalid: {error}"
        ))
    })?;
    if repository.scheme() != "https" || repository.host_str().is_none() {
        return Err(LauncherError::InvalidRemoteData(format!(
            "Maven repository '{repository}' must use absolute HTTPS"
        )));
    }
    if !repository.path().ends_with('/') {
        repository.set_path(&format!("{}/", repository.path()));
    }
    let url = repository.join(&path).map_err(|error| {
        LauncherError::InvalidRemoteData(format!(
            "Maven artifact '{coordinate}' cannot be resolved: {error}"
        ))
    })?;
    Ok((path, url.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_produce_standard_paths_without_traversal() {
        assert_eq!(
            artifact_path("com.example:demo:1.0:natives-windows@zip", None).unwrap(),
            "com/example/demo/1.0/demo-1.0-natives-windows.zip"
        );
        assert_eq!(
            artifact_url("https://maven.example/repository", "a.b:c:1", None)
                .unwrap()
                .1,
            "https://maven.example/repository/a/b/c/1/c-1.jar"
        );
        assert!(artifact_path("../evil:demo:1.0", None).is_err());
    }
}
