//! Commands for contacts.

use anyhow::Result;
use bitlog_core::{Contact, ContactSlug, EditError, ProjectSlug, Vault};

use crate::table::table;

/// A table of the contacts by name, the archived ones only if `all`: slug,
/// name, company, the projects they work on, and whether they are archived.
pub fn format_contacts(vault: &Vault, all: bool) -> String {
    let rows: Vec<[String; 5]> = vault
        .contacts()
        .iter()
        .filter(|contact| all || !contact.archived)
        .map(|contact| {
            let projects: Vec<&str> = contact
                .roles
                .iter()
                .map(|(project, _)| project.as_str())
                .collect();
            [
                contact.slug.to_string(),
                contact.name.clone(),
                contact.company.clone(),
                projects.join(", "),
                if contact.archived { "archived" } else { "" }.to_owned(),
            ]
        })
        .collect();
    table(&rows, &[])
}

/// The fields of the contact `slug` that have a value, one per line, then
/// the projects they work on with their roles.
pub fn format_contact(vault: &Vault, slug: &ContactSlug) -> Result<String> {
    let contact = vault
        .contact(slug)
        .ok_or_else(|| EditError::UnknownContact(slug.clone()))?;
    let mut rows: Vec<[String; 2]> = Vec::new();
    let mut add = |label: &str, value: &str| {
        if !value.is_empty() {
            // Notes may span lines, which would break the columns.
            let value: Vec<&str> = value.lines().collect();
            rows.push([label.to_owned(), value.join(" ")]);
        }
    };
    add("Name", &contact.name);
    add("Company", &contact.company);
    add("Hours", &contact.hours);
    add("Time zone", &contact.timezone);
    let birthday = contact.birthday.map(|date| date.to_string());
    add("Birthday", birthday.as_deref().unwrap_or_default());
    for (label, number) in &contact.phones {
        add("Phone", &format!("{number} ({label})"));
    }
    for (label, address) in &contact.emails {
        add("Email", &format!("{address} ({label})"));
    }
    for (label, text) in &contact.fields {
        add(label, text);
    }
    add("Note", &contact.note);
    add("Archived", if contact.archived { "yes" } else { "" });
    let mut text = table(&rows, &[]);
    let roles: Vec<[String; 3]> = contact
        .roles
        .iter()
        .map(|(project, role)| {
            [
                project.to_string(),
                vault.project_name(project).to_owned(),
                role.clone(),
            ]
        })
        .collect();
    if !roles.is_empty() {
        text.push('\n');
        text += &table(&roles, &[]);
    }
    Ok(text)
}

/// Changes to a contact; `None` keeps a value as it is.
pub struct ContactChanges {
    pub name: Option<String>,
    pub company: Option<String>,
    pub archived: Option<bool>,
    /// Projects with the role to set there, `None` to remove the project.
    pub roles: Vec<(ProjectSlug, Option<String>)>,
}

pub fn add_contact(vault: &mut Vault, slug: ContactSlug, changes: ContactChanges) -> Result<()> {
    let name = changes.name.clone().unwrap_or_else(|| slug.to_string());
    check_projects(vault, &changes)?;
    let mut contact = Contact::new(slug, &name);
    apply(&mut contact, changes);
    let slug = vault.add_contact(contact)?.slug.clone();
    println!("Added contact {slug}");
    Ok(())
}

pub fn edit_contact(vault: &mut Vault, slug: &ContactSlug, changes: ContactChanges) -> Result<()> {
    // Checked first, as changing the contact borrows the vault.
    check_projects(vault, &changes)?;
    vault.update_contact(slug, |contact| {
        apply(contact, changes);
        Ok(())
    })?;
    Ok(())
}

/// Fails for a role in a project the vault lacks. Removing one is fine.
fn check_projects(vault: &Vault, changes: &ContactChanges) -> Result<(), EditError> {
    match changes
        .roles
        .iter()
        .find(|(project, role)| role.is_some() && vault.project(project).is_none())
    {
        Some((project, _)) => Err(EditError::UnknownProject(project.clone())),
        None => Ok(()),
    }
}

fn apply(contact: &mut Contact, changes: ContactChanges) {
    if let Some(name) = changes.name {
        contact.name = name;
    }
    if let Some(company) = changes.company {
        contact.company = company;
    }
    if let Some(archived) = changes.archived {
        contact.archived = archived;
    }
    for (project, role) in &changes.roles {
        contact.set_role(project, role.as_deref());
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn sample_vault() -> Vault {
        Vault::open(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault"))
            .unwrap()
    }

    #[test]
    fn sample_contacts() {
        let vault = sample_vault();
        assert_eq!(
            format_contacts(&vault, false),
            "alex  Alex Novak  Hosting Partner s.r.o.  infra, webshop\n\
             kim   Kim Meyer   Example GmbH            webshop\n"
        );
        assert!(
            format_contacts(&vault, true)
                .ends_with("sam   Sam Berg                                            archived\n")
        );
    }

    #[test]
    fn show_contact() {
        let vault = sample_vault();
        assert_eq!(
            format_contact(&vault, &"kim".parse().unwrap()).unwrap(),
            "Name       Kim Meyer\n\
             Company    Example GmbH\n\
             Hours      Mon–Thu, 9–15\n\
             Time zone  Europe/Berlin\n\
             Birthday   1990-05-17\n\
             Phone      +49 30 1234567 (work)\n\
             Phone      +49 170 1234567 (mobile)\n\
             Email      kim.meyer@example.com (work)\n\
             Office     Berlin, 3rd floor, 3.14\n\
             GitHub     https://github.com/kimm\n\
             Note       Prefers calls in the morning.\n\
             \n\
             webshop  Webshop  Product Owner\n"
        );
        assert!(format_contact(&vault, &"nope".parse().unwrap()).is_err());
    }
}
