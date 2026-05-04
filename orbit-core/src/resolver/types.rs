

/// 包标识符
pub type PackageId = String;

#[derive(Debug, Clone)]
pub struct CandidateVersion {
    pub jar_version: String,
    pub deps: Vec<(String, String, bool)>,
    pub implanted: Vec<ImplantedCandidate>,
}

#[derive(Debug, Clone)]
pub struct ImplantedCandidate {
    pub mod_id: String,
    pub version: String,
    pub deps: Vec<(String, String, bool)>,
}
