//! Review state is a projection, never an implicit lifecycle transition.
//! Implements [[RFC-0011:C-REVIEW]], [[RFC-0011:C-TRUST]].
use super::{Target, records::History, source::Source};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

fn identity(actor: &Value) -> String {
    actor["agentId"]
        .as_str()
        .map(|id| format!("agent:{id}"))
        .unwrap_or_else(|| actor["subjectId"].as_str().unwrap_or_default().to_owned())
}
fn role(actor: &Value, roles: &[&str]) -> bool {
    actor["role"]
        .as_str()
        .is_some_and(|role| roles.contains(&role))
        || actor["roles"].as_array().is_some_and(|values| {
            values
                .iter()
                .any(|value| value.as_str().is_some_and(|role| roles.contains(&role)))
        })
}
fn matches(event: &Value, target: &Target) -> bool {
    serde_json::from_value::<Target>(event["target"].clone())
        .is_ok_and(|candidate| candidate == *target)
}
fn authorship(source: &Source, target: &Target, history: &[History]) -> Value {
    let mut declared = BTreeMap::new();
    let mut trusted = BTreeMap::new();
    let mut ids = Vec::new();
    if let Some(actor) = &source.created_by {
        declared.insert(identity(actor), actor.clone());
    }
    for entry in history {
        let Some(event) = &entry.event else { continue };
        if event["type"] != "authorship" || !matches(event, target) {
            continue;
        }
        ids.push(entry.event_id.clone());
        let confirmed = entry.verification == "verified"
            && (role(&event["actor"], &["owner", "admin"])
                || event["payload"]["automatic"] == true);
        for actor in event["payload"]["authors"].as_array().into_iter().flatten() {
            declared.insert(identity(actor), actor.clone());
            if confirmed {
                trusted.insert(identity(actor), actor.clone());
            }
        }
    }
    let confirmed = !trusted.is_empty() && declared.keys().all(|id| trusted.contains_key(id));
    json!({"confirmed":confirmed,"authors":declared.into_values().collect::<Vec<_>>(),"eventIds":ids})
}

pub(super) fn render(view: &Value) {
    println!(
        "# {} — collaboration",
        view["target"]["id"].as_str().unwrap_or_default()
    );
    println!(
        "\nAuthors ({})",
        if view["authorship"]["confirmed"] == true {
            "confirmed"
        } else {
            "declared / unconfirmed"
        }
    );
    for author in view["authorship"]["authors"]
        .as_array()
        .into_iter()
        .flatten()
    {
        println!(
            "- {} ({})",
            author["name"].as_str().unwrap_or_default(),
            author["subjectId"].as_str().unwrap_or_default()
        );
    }
    println!(
        "\nApprovals: {}; unresolved change requests: {}\n",
        view["reviewState"]["approvedCount"], view["reviewState"]["blockingCount"]
    );
    for review in view["reviews"].as_array().into_iter().flatten() {
        println!(
            "## {} — {} [{}; {}]\n\n{}\n",
            review["author"]["name"].as_str().unwrap_or_default(),
            review["verdict"].as_str().unwrap_or_default(),
            review["validity"]["state"].as_str().unwrap_or_default(),
            review["provenance"].as_str().unwrap_or_default(),
            review["reason"].as_str().unwrap_or_default()
        );
        println!(
            "Event: {}\n",
            review["eventId"].as_str().unwrap_or_default()
        );
    }
}
pub(super) fn view(
    source: Source,
    history: Vec<History>,
    head: Option<&str>,
    clean: bool,
) -> Value {
    let authors = authorship(&source, &source.target, &history);
    let mut reviews = Vec::new();
    for entry in &history {
        let Some(event) = &entry.event else { continue };
        if event["type"] != "review" && event["type"] != "comment" {
            continue;
        }
        let comment = event["type"] == "comment";
        let scope = if comment {
            "artifact"
        } else {
            event["payload"]["scope"].as_str().unwrap_or("artifact")
        };
        let verdict = if comment {
            "comment"
        } else {
            event["payload"]["verdict"].as_str().unwrap_or("comment")
        };
        let reason = if comment {
            &event["payload"]["body"]
        } else {
            &event["payload"]["reason"]
        };
        let mut current = source.target.clone();
        if scope == "code" {
            current.commit = head.map(str::to_owned);
        }
        let current_authors = authorship(&source, &current, &history);
        let current_match = matches(event, &current);
        let resolution = history.iter().find(|candidate| {
            candidate.verification == "verified"
                && candidate.event.as_ref().is_some_and(|value| {
                    value["type"] == "resolution"
                        && value["payload"]["reviewId"] == event["eventId"]
                        && (role(&value["actor"], &["owner", "arbiter", "admin"])
                            || identity(&value["actor"]) == identity(&event["actor"]))
                })
        });
        let verified = entry.verification == "verified";
        let ineffective = if !verified {
            Some("The identity or signing key has not been independently trusted.")
        } else if resolution.is_some() {
            Some("This conclusion was explicitly resolved.")
        } else if verdict == "approved"
            && !role(&event["actor"], &["owner", "arbiter", "admin", "reviewer"])
        {
            Some("The actor does not declare a review role.")
        } else if verdict == "approved" && !current_match {
            Some("The reviewed source or code commit has changed.")
        } else if verdict == "approved" && scope == "code" && !clean {
            Some("Uncommitted files cannot be represented as reviewed HEAD code.")
        } else if verdict == "approved" && current_authors["confirmed"] != true {
            Some("The complete authorship of this exact revision has not been confirmed.")
        } else if verdict == "approved"
            && current_authors["authors"]
                .as_array()
                .into_iter()
                .flatten()
                .any(|a| identity(a) == identity(&event["actor"]))
        {
            Some("An author cannot approve their own revision.")
        } else {
            None
        };
        let state = if !verified {
            "unverified"
        } else if resolution.is_some() {
            "resolved"
        } else if current_match {
            "current"
        } else {
            "stale"
        };
        let mut review = json!({"eventId":event["eventId"],"createdAt":event["createdAt"],"author":event["actor"],"scope":scope,"target":event["target"],
            "verdict":verdict,"reason":reason,"verification":entry.verification,"provenance":entry.provenance,
            "validity":{"state":state},"resolvedBy":resolution.map(|r| &r.event_id),"effective":verdict!="comment" && ineffective.is_none()});
        if let Some(reason) = ineffective {
            review["ineffectiveReason"] = json!(reason);
            review["validity"]["reason"] = json!(reason);
        }
        if comment {
            review["replyTo"] = event["payload"]["replyTo"].clone();
        }
        reviews.push(review);
    }
    let approved = reviews
        .iter()
        .filter(|r| r["effective"] == true && r["verdict"] == "approved")
        .map(|r| identity(&r["author"]))
        .collect::<BTreeSet<_>>()
        .len();
    let blocking = reviews
        .iter()
        .filter(|r| r["effective"] == true && r["verdict"] == "changes_requested")
        .count();
    let mut reasons = Vec::new();
    if authors["confirmed"] != true {
        reasons.push("The complete authorship of this revision is not confirmed.");
    }
    if blocking > 0 {
        reasons.push("Unresolved trusted reviews request changes.");
    }
    json!({"target":source.target,"history":history,"authorship":authors,"reviews":reviews,
        "reviewState":{"approvedCount":approved,"blockingCount":blocking,"authorsConfirmed":authors["confirmed"],"reasons":reasons}})
}
