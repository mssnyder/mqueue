//! Google People API client for syncing contacts.
//!
//! Fetches the user's Google contacts for autocomplete in compose.
//! Uses the `people.connections.list` endpoint with `emailAddresses` and `names` fields.

use serde::Deserialize;
use tracing::{debug, info, warn};

use crate::error::{MqError, Result};

const PEOPLE_API_URL: &str = "https://people.googleapis.com/v1/people/me/connections";

/// A contact fetched from the Google People API.
#[derive(Debug, Clone)]
pub struct Contact {
    pub resource_id: String,
    pub display_name: Option<String>,
    pub email: String,
}

#[derive(Deserialize)]
struct PeopleResponse {
    connections: Option<Vec<Person>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(rename = "totalItems")]
    total_items: Option<u32>,
}

#[derive(Deserialize)]
struct Person {
    #[serde(rename = "resourceName")]
    resource_name: Option<String>,
    names: Option<Vec<Name>>,
    #[serde(rename = "emailAddresses")]
    email_addresses: Option<Vec<EmailAddress>>,
}

#[derive(Deserialize)]
struct Name {
    #[serde(rename = "displayName")]
    display_name: Option<String>,
}

#[derive(Deserialize)]
struct EmailAddress {
    value: Option<String>,
}

/// Fetch all contacts from the user's Google account.
///
/// Paginates through the People API to get all connections with email addresses.
pub async fn fetch_contacts(access_token: &str) -> Result<Vec<Contact>> {
    let client = reqwest::Client::new();
    let mut contacts = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let mut req = client
            .get(PEOPLE_API_URL)
            .bearer_auth(access_token)
            .query(&[
                ("personFields", "names,emailAddresses"),
                ("pageSize", "1000"),
                ("sortOrder", "FIRST_NAME_ASCENDING"),
            ]);

        if let Some(ref token) = page_token {
            req = req.query(&[("pageToken", token.as_str())]);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| MqError::Other(anyhow::anyhow!("People API request failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MqError::Other(anyhow::anyhow!(
                "People API error {status}: {body}"
            )));
        }

        let data: PeopleResponse = resp
            .json()
            .await
            .map_err(|e| MqError::Other(anyhow::anyhow!("People API parse error: {e}")))?;

        if let Some(total) = data.total_items {
            debug!(total, "People API total contacts");
        }

        if let Some(connections) = data.connections {
            for person in connections {
                let display_name = person
                    .names
                    .as_ref()
                    .and_then(|names| names.first())
                    .and_then(|n| n.display_name.clone());

                let resource_id = person.resource_name.unwrap_or_default();

                if let Some(emails) = person.email_addresses {
                    for email_addr in emails {
                        if let Some(email) = email_addr.value {
                            if !email.is_empty() {
                                contacts.push(Contact {
                                    resource_id: resource_id.clone(),
                                    display_name: display_name.clone(),
                                    email,
                                });
                            }
                        }
                    }
                }
            }
        }

        match data.next_page_token {
            Some(token) if !token.is_empty() => {
                page_token = Some(token);
            }
            _ => break,
        }
    }

    info!(count = contacts.len(), "Fetched contacts from Google People API");
    Ok(contacts)
}

/// Fetch contacts, returning an empty list on auth errors (scope not granted).
///
/// This is a graceful wrapper for sync — if the user hasn't granted the
/// contacts scope, we just skip contacts sync without failing.
pub async fn fetch_contacts_graceful(access_token: &str) -> Vec<Contact> {
    match fetch_contacts(access_token).await {
        Ok(contacts) => contacts,
        Err(e) => {
            warn!("Contacts sync failed (scope may not be granted): {e}");
            Vec::new()
        }
    }
}
