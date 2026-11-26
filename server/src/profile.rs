use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Link {
    pub label: String,
    pub href: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Metric {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub step: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Group {
    pub name: String,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Job {
    pub id: String,
    pub title: String,
    pub org: String,
    pub period: String,
    pub intro: String,
    pub groups: Vec<Group>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub id: String,
    pub title: String,
    pub context: String,
    pub action: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SkillArea {
    pub area: String,
    pub items: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Project {
    pub name: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub href: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Education {
    pub title: String,
    pub org: String,
    pub year: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub role: String,
    pub tagline: String,
    pub location: String,
    pub summary: Vec<String>,
    pub links: Vec<Link>,
    pub metrics: Vec<Metric>,
    pub pipeline: Vec<Stage>,
    pub experience: Vec<Job>,
    pub cases: Vec<Case>,
    pub skills: Vec<SkillArea>,
    pub projects: Vec<Project>,
    pub education: Vec<Education>,
    pub credentials: Vec<String>,
}

pub const DEFAULT_PROFILE: &str = include_str!("../content/profile.json");

/// links people can safely click: no javascript:, data: and the like
pub fn safe_href(h: &str) -> bool {
    let h = h.trim().to_ascii_lowercase();
    h.starts_with("https://") || h.starts_with("http://") || h.starts_with("mailto:")
}

impl Profile {
    pub fn from_json(raw: &str) -> Result<Self, String> {
        let p: Profile = serde_json::from_str(raw).map_err(|e| e.to_string())?;
        p.validate()?;
        Ok(p)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("profile name is empty".into());
        }
        if let Some(l) = self.links.iter().find(|l| !safe_href(&l.href)) {
            return Err(format!("link {:?}: only https, http and mailto links are allowed", l.label));
        }
        if let Some(p) = self.projects.iter().find(|p| p.href.as_deref().is_some_and(|h| !safe_href(h))) {
            return Err(format!("project {:?}: only https, http and mailto links are allowed", p.name));
        }
        let mut ids = std::collections::HashSet::new();
        if let Some(j) = self.experience.iter().find(|j| !ids.insert(j.id.as_str())) {
            return Err(format!("job id {:?} is used twice", j.id));
        }
        Ok(())
    }

    /// drop anything unsafe that reached the database some other way
    pub fn sanitised(mut self) -> Self {
        self.links.retain(|l| {
            let ok = safe_href(&l.href);
            if !ok {
                tracing::warn!(label = %l.label, "profile link dropped: only https, http and mailto are allowed");
            }
            ok
        });
        for p in &mut self.projects {
            if p.href.as_deref().is_some_and(|h| !safe_href(h)) {
                tracing::warn!(project = %p.name, "project link dropped: only https, http and mailto are allowed");
                p.href = None;
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_is_valid() {
        let p = Profile::from_json(DEFAULT_PROFILE).unwrap();
        assert!(!p.experience.is_empty() && !p.metrics.is_empty());
    }

    #[test]
    fn unsafe_links_are_refused() {
        let mut p = Profile::from_json(DEFAULT_PROFILE).unwrap();
        p.links.push(Link { label: "x".into(), href: " JavaScript:alert(1)".into(), text: "x".into() });
        assert!(p.validate().is_err());
        assert!(p.clone().sanitised().links.iter().all(|l| safe_href(&l.href)));

        let mut q = Profile::from_json(DEFAULT_PROFILE).unwrap();
        q.projects[0].href = Some("javascript:alert(1)".into());
        assert!(q.validate().is_err());
        assert_eq!(q.sanitised().projects[0].href, None);
    }

    #[test]
    fn unknown_fields_are_refused() {
        assert!(Profile::from_json(r#"{"name":"A","admin":true}"#).is_err());
    }
}
