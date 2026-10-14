//! Contacts, `contacts/<slug>.toml`, and their pictures, `contacts/<slug>.jpg`.

use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use serde::Deserialize;
use toml_edit::DocumentMut;

use crate::error::{ReadError, SaveError};
use crate::file::{
    FORMAT, check_format, content_hash, parse_text, parse_toml, read_file, read_folder, read_text,
    write_atomic,
};
use crate::toml_values::{labeled_texts, local_date, set, set_date, set_labeled_texts, set_text};
use crate::{ContactSlug, EditError, InvalidId, ProjectSlug, Vault};

/// The labels of phone numbers that the app translates. Any other label is
/// shown as written.
pub const PHONE_LABELS: [&str; 3] = ["work", "mobile", "private"];

/// The labels of e-mail addresses that the app translates.
pub const EMAIL_LABELS: [&str; 2] = ["work", "private"];

/// A person, with the projects they work on. Texts that are not given are
/// empty.
#[derive(Debug, Clone)]
pub struct Contact {
    /// The name of the file, without `.toml`.
    pub slug: ContactSlug,
    pub name: String,
    pub company: String,
    /// When the person works, free text.
    pub hours: String,
    /// An IANA name such as `Europe/Berlin`, which is not checked.
    pub timezone: String,
    pub birthday: Option<NaiveDate>,
    /// Plain text, not Markdown.
    pub note: String,
    /// Labels and numbers, the preferred first.
    pub phones: Vec<(String, String)>,
    /// Labels and addresses, the preferred first.
    pub emails: Vec<(String, String)>,
    /// Labels and texts of all other fields, such as an office or a profile.
    pub fields: Vec<(String, String)>,
    /// The projects the person works on, with their role there, which may
    /// be empty.
    pub roles: Vec<(ProjectSlug, String)>,
    pub archived: bool,
    /// The file as read, so that writing keeps comments, formatting and
    /// fields this version does not know.
    document: DocumentMut,
    /// Of the file as read, `None` for new contacts.
    hash: Option<u64>,
}

#[derive(Deserialize)]
struct ContactFile {
    format: u32,
    name: Option<String>,
    company: Option<String>,
    hours: Option<String>,
    timezone: Option<String>,
    #[serde(default, deserialize_with = "local_date")]
    birthday: Option<NaiveDate>,
    note: Option<String>,
    #[serde(default)]
    archived: bool,
}

/// Where contacts and their pictures live in `vault`.
fn contacts_folder(vault: &Path) -> PathBuf {
    vault.join("contacts")
}

impl Contact {
    /// A new contact with nothing but its name.
    pub fn new(slug: ContactSlug, name: &str) -> Self {
        Self {
            slug,
            name: name.to_owned(),
            company: String::new(),
            hours: String::new(),
            timezone: String::new(),
            birthday: None,
            note: String::new(),
            phones: Vec::new(),
            emails: Vec::new(),
            fields: Vec::new(),
            roles: Vec::new(),
            archived: false,
            document: DocumentMut::new(),
            hash: None,
        }
    }

    /// Where the file of the contact `slug` lives in `vault`.
    pub fn path(vault: &Path, slug: &ContactSlug) -> PathBuf {
        contacts_folder(vault).join(format!("{slug}.toml"))
    }

    /// Where the picture of the contact `slug` lives in `vault`, if it has
    /// one.
    pub fn image_path(vault: &Path, slug: &ContactSlug) -> PathBuf {
        contacts_folder(vault).join(format!("{slug}.jpg"))
    }

    /// The role of the person in the project `project`, if they work on it.
    pub fn role(&self, project: &ProjectSlug) -> Option<&str> {
        self.roles
            .iter()
            .find(|(slug, _)| slug == project)
            .map(|(_, role)| role.as_str())
    }

    /// Sets the role of the person in the project `project`, adding the
    /// project after the others, or with `None` removes the project.
    pub fn set_role(&mut self, project: &ProjectSlug, role: Option<&str>) {
        let index = self.roles.iter().position(|(slug, _)| slug == project);
        match (index, role) {
            (Some(index), Some(role)) => self.roles[index].1 = role.to_owned(),
            (None, Some(role)) => self.roles.push((project.clone(), role.to_owned())),
            (Some(index), None) => {
                self.roles.remove(index);
            }
            (None, None) => {}
        }
    }

    /// Loads all contacts of `vault`, sorted by slug, each or why it cannot
    /// be read.
    ///
    /// Only files below `contacts/` named by a valid slug and `.toml` are
    /// contacts, which leaves out sync conflict copies.
    pub(crate) fn load_all(vault: &Path) -> Result<Vec<Result<Self, ReadError>>, ReadError> {
        let mut slugs: Vec<ContactSlug> = read_folder(&contacts_folder(vault))?
            .iter()
            .filter_map(|entry| {
                let name = entry.file_name();
                name.to_str()?.strip_suffix(".toml")?.parse().ok()
            })
            .collect();
        slugs.sort();
        Ok(slugs
            .into_iter()
            .map(|slug| read_file(&Self::path(vault, &slug), |text| Self::parse(slug, text)))
            .collect())
    }

    /// The pictures in `vault` whose contact has no file.
    pub(crate) fn lone_images(vault: &Path) -> Result<Vec<PathBuf>, ReadError> {
        let mut images: Vec<PathBuf> = read_folder(&contacts_folder(vault))?
            .iter()
            .filter_map(|entry| {
                let name = entry.file_name();
                let slug: ContactSlug = name.to_str()?.strip_suffix(".jpg")?.parse().ok()?;
                (!Self::path(vault, &slug).exists()).then(|| entry.path())
            })
            .collect();
        images.sort();
        Ok(images)
    }

    fn read(vault: &Path, slug: ContactSlug, text: &str) -> Result<Self, ReadError> {
        parse_text(&Self::path(vault, &slug), text, |text| {
            Self::parse(slug, text)
        })
    }

    fn parse(slug: ContactSlug, text: &str) -> Result<Self, String> {
        let (file, document, hash): (ContactFile, DocumentMut, _) = parse_toml(text)?;
        check_format(file.format)?;
        let table = document.as_table();
        let roles = labeled_texts(table, "projects")?
            .into_iter()
            .map(|(project, role)| {
                Ok((
                    project.parse().map_err(|err: InvalidId| err.to_string())?,
                    role,
                ))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self {
            name: file.name.unwrap_or_else(|| slug.to_string()),
            slug,
            company: file.company.unwrap_or_default(),
            hours: file.hours.unwrap_or_default(),
            timezone: file.timezone.unwrap_or_default(),
            birthday: file.birthday,
            note: file.note.unwrap_or_default(),
            phones: labeled_texts(table, "phone")?,
            emails: labeled_texts(table, "email")?,
            fields: labeled_texts(table, "fields")?,
            roles,
            archived: file.archived,
            document,
            hash: Some(hash),
        })
    }

    /// The content of the contact's file. Comments, formatting and unknown
    /// fields of the file this contact was read from are kept, unchanged
    /// values are written exactly as they were.
    pub fn to_toml(&self) -> String {
        let mut document = self.document.clone();
        let table = document.as_table_mut();
        set(table, "format", i64::from(FORMAT).into());
        set(table, "name", self.name.as_str().into());
        set_text(table, "company", &self.company);
        set_text(table, "hours", &self.hours);
        set_text(table, "timezone", &self.timezone);
        set_date(table, "birthday", self.birthday);
        set_text(table, "note", &self.note);
        if self.archived {
            set(table, "archived", true.into());
        } else {
            table.remove("archived");
        }
        set_labeled_texts(table, "phone", &self.phones);
        set_labeled_texts(table, "email", &self.emails);
        set_labeled_texts(table, "fields", &self.fields);
        let roles: Vec<(String, String)> = self
            .roles
            .iter()
            .map(|(project, role)| (project.to_string(), role.clone()))
            .collect();
        set_labeled_texts(table, "projects", &roles);
        document.to_string()
    }
}

impl Vault {
    /// The contacts that can be read, sorted by name.
    pub fn contacts(&self) -> &[Contact] {
        &self.contacts
    }

    pub fn contact(&self, slug: &ContactSlug) -> Option<&Contact> {
        self.contacts.iter().find(|contact| contact.slug == *slug)
    }

    /// Reads the contacts again, as after removing one.
    pub fn reload_contacts(&mut self) -> Result<(), ReadError> {
        self.contacts = Contact::load_all(self.root())?
            .into_iter()
            .filter_map(Result::ok)
            .collect();
        self.sort_contacts();
        Ok(())
    }

    fn sort_contacts(&mut self) {
        self.contacts
            .sort_by_cached_key(|contact| (contact.name.to_lowercase(), contact.slug.clone()));
    }

    /// Saves the new contact `contact` and adds it to the vault.
    pub fn add_contact(&mut self, contact: Contact) -> Result<&Contact, SaveError> {
        let path = Contact::path(self.root(), &contact.slug);
        if self.contact(&contact.slug).is_some() || path.exists() {
            return Err(EditError::ContactExists(contact.slug).into());
        }
        let text = contact.to_toml();
        let saved = Contact::read(self.root(), contact.slug.clone(), &text)
            .expect("new contacts are valid");
        self.write(&path, &text)?;
        self.contacts.push(saved);
        self.sort_contacts();
        Ok(self
            .contact(&contact.slug)
            .expect("the contact was just added"))
    }

    /// Applies `change` to the contact `slug` of this vault and saves it.
    ///
    /// If the file was changed elsewhere since it was read, `change` is applied
    /// to the contact as it is now instead, so that both changes are kept.
    pub fn update_contact(
        &mut self,
        slug: &ContactSlug,
        change: impl FnOnce(&mut Contact) -> Result<(), EditError>,
    ) -> Result<&Contact, SaveError> {
        let index = self
            .contacts
            .iter()
            .position(|contact| contact.slug == *slug)
            .ok_or_else(|| EditError::UnknownContact(slug.clone()))?;
        let path = Contact::path(self.root(), slug);
        let current = read_text(&path)?;
        let mut contact = if self.contacts[index].hash == Some(content_hash(&current)) {
            self.contacts[index].clone()
        } else {
            Contact::read(self.root(), slug.clone(), &current)?
        };
        change(&mut contact)?;
        let text = contact.to_toml();
        let saved =
            Contact::read(self.root(), slug.clone(), &text).expect("changes keep a contact valid");
        // An unchanged contact comes out byte-identical.
        if current != text {
            self.write(&path, &text)?;
        }
        self.contacts[index] = saved;
        // The name may place it elsewhere.
        self.sort_contacts();
        Ok(self.contact(slug).expect("the contact was just saved"))
    }

    /// Saves `jpeg` as the picture of the contact `slug`, replacing the one
    /// it has.
    pub fn save_contact_image(&self, slug: &ContactSlug, jpeg: &[u8]) -> Result<(), SaveError> {
        if self.contact(slug).is_none() {
            return Err(EditError::UnknownContact(slug.clone()).into());
        }
        write_atomic(&Contact::image_path(self.root(), slug), jpeg)
    }

    /// Points the roles in the project `from` to the project `to`.
    pub(crate) fn rename_roles(
        &mut self,
        from: &ProjectSlug,
        to: &ProjectSlug,
    ) -> Result<(), SaveError> {
        let slugs: Vec<ContactSlug> = self
            .contacts
            .iter()
            .filter(|contact| contact.role(from).is_some())
            .map(|contact| contact.slug.clone())
            .collect();
        for slug in slugs {
            self.update_contact(&slug, |contact| {
                for (project, _) in &mut contact.roles {
                    if project == from {
                        *project = to.clone();
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use crate::file::sample_copy;

    fn slug(value: &str) -> ContactSlug {
        value.parse().unwrap()
    }

    fn project(value: &str) -> ProjectSlug {
        value.parse().unwrap()
    }

    fn sample_vault() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/sample-vault")
    }

    fn texts(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(label, text)| ((*label).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn sample_contacts() {
        let vault = Vault::open(&sample_vault()).unwrap();
        let names: Vec<&str> = vault.contacts().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Alex Novak", "Kim Meyer", "Sam Berg"]);
        let kim = vault.contact(&slug("kim")).unwrap();
        assert_eq!(kim.company, "Example GmbH");
        assert_eq!(kim.timezone, "Europe/Berlin");
        assert_eq!(kim.birthday, NaiveDate::from_ymd_opt(1990, 5, 17));
        assert_eq!(
            kim.phones,
            texts(&[("work", "+49 30 1234567"), ("mobile", "+49 170 1234567")])
        );
        assert_eq!(kim.emails, texts(&[("work", "kim.meyer@example.com")]));
        assert_eq!(
            kim.fields[0],
            texts(&[("Office", "Berlin, 3rd floor, 3.14")])[0]
        );
        assert_eq!(kim.role(&project("webshop")), Some("Product Owner"));
        assert_eq!(kim.role(&project("infra")), None);
        assert!(!kim.archived);
        assert!(vault.contact(&slug("sam")).unwrap().archived);
    }

    #[test]
    fn unchanged_files_are_written_byte_identical() {
        for contact in Contact::load_all(&sample_vault()).unwrap() {
            let contact = contact.unwrap();
            let text = fs::read_to_string(Contact::path(&sample_vault(), &contact.slug)).unwrap();
            assert_eq!(contact.to_toml(), text, "{}", contact.slug);
        }
    }

    #[test]
    fn new_contact_roundtrip() {
        let mut contact = Contact::new(slug("kim"), "Kim");
        contact.company = "Example".to_owned();
        contact.birthday = NaiveDate::from_ymd_opt(1990, 5, 17);
        contact.phones = texts(&[("mobile", "+49 170"), ("Home office", "+49 30")]);
        contact.set_role(&project("webshop"), Some("Product Owner"));
        contact.set_role(&project("infra"), Some(""));
        contact.archived = true;
        let text = contact.to_toml();
        assert_eq!(
            text,
            "format = 1\nname = \"Kim\"\ncompany = \"Example\"\nbirthday = 1990-05-17\narchived = true\n\n\
             [phone]\nmobile = \"+49 170\"\n\"Home office\" = \"+49 30\"\n\n\
             [projects]\nwebshop = \"Product Owner\"\ninfra = \"\"\n"
        );
        let read = Contact::parse(slug("kim"), &text).unwrap();
        assert_eq!(read.to_toml(), text);
        assert_eq!(read.roles, contact.roles);
    }

    #[test]
    fn writing_keeps_comments_and_order() {
        let mut contact = Contact::parse(
            slug("kim"),
            "format = 1\nname = \"Kim\"\nphone = { work = \"1\" }\nnickname = \"K\"\n\n\
             [email]\n# Read daily.\nwork = \"a@example.com\" # main\nprivate = \"b@example.com\"\n",
        )
        .unwrap();
        assert_eq!(contact.phones, texts(&[("work", "1")]));
        contact.emails = texts(&[("private", "b@example.com"), ("work", "c@example.com")]);
        contact.phones.clear();
        contact.company = "Example".to_owned();
        assert_eq!(
            contact.to_toml(),
            "format = 1\nname = \"Kim\"\nnickname = \"K\"\ncompany = \"Example\"\n\n\
             [email]\nprivate = \"b@example.com\"\n# Read daily.\nwork = \"c@example.com\" # main\n"
        );
    }

    #[test]
    fn defaults_and_invalid_files() {
        let contact = Contact::parse(slug("kim"), "format = 1").unwrap();
        assert_eq!(contact.name, "kim");
        assert!(contact.phones.is_empty() && contact.roles.is_empty());
        assert_eq!(contact.to_toml(), "format = 1\nname = \"kim\"\n");
        for text in [
            "",
            "format = 2",
            "format = 1\nphone = \"123\"",
            "format = 1\n[phone]\nwork = 123",
            "format = 1\n[projects]\nWeb = \"Owner\"",
            "format = 1\nbirthday = \"1990-05-17\"",
            "format = 1\narchived = \"yes\"",
        ] {
            assert!(Contact::parse(slug("a"), text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn roles() {
        let mut contact = Contact::new(slug("kim"), "Kim");
        contact.set_role(&project("webshop"), Some("Owner"));
        contact.set_role(&project("infra"), Some("Ops"));
        contact.set_role(&project("webshop"), Some("Lead"));
        assert_eq!(
            contact.roles,
            [
                (project("webshop"), "Lead".to_owned()),
                (project("infra"), "Ops".to_owned())
            ]
        );
        contact.set_role(&project("webshop"), None);
        contact.set_role(&project("nope"), None);
        assert_eq!(contact.roles, [(project("infra"), "Ops".to_owned())]);
    }

    #[test]
    fn unreadable_contacts_are_left_out() {
        let (_dir, mut vault) = sample_copy();
        let path = Contact::path(vault.root(), &slug("alex"));
        fs::write(&path, "format = 1\nname = [").unwrap();
        fs::write(vault.root().join("contacts/Not A Slug.toml"), "").unwrap();
        vault.reload_contacts().unwrap();
        let slugs: Vec<&str> = vault.contacts().iter().map(|c| c.slug.as_str()).collect();
        assert_eq!(slugs, ["kim", "sam"]);
        assert_eq!(Vault::open(vault.root()).unwrap().contacts().len(), 2);
    }

    #[test]
    fn add_and_update_contact() {
        let (_dir, mut vault) = sample_copy();
        let contact = Contact::new(slug("ada"), "Ada Lovelace");
        vault.add_contact(contact.clone()).unwrap();
        assert_eq!(vault.contacts()[0].slug, slug("ada"));
        assert!(matches!(
            vault.add_contact(contact),
            Err(SaveError::Edit(EditError::ContactExists(_)))
        ));

        // Changed elsewhere meanwhile: both changes are kept.
        let path = Contact::path(vault.root(), &slug("kim"));
        let external = fs::read_to_string(&path)
            .unwrap()
            .replace("Example GmbH", "Example AG");
        fs::write(&path, &external).unwrap();
        let kim = vault
            .update_contact(&slug("kim"), |kim| {
                kim.name = "Kim M.".to_owned();
                Ok(())
            })
            .unwrap();
        assert_eq!(kim.company, "Example AG");
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            external.replace("\"Kim Meyer\"", "\"Kim M.\"")
        );
        assert!(matches!(
            vault.update_contact(&slug("nope"), |_| Ok(())),
            Err(SaveError::Edit(EditError::UnknownContact(_)))
        ));
    }

    #[test]
    fn images() {
        let (_dir, vault) = sample_copy();
        vault.save_contact_image(&slug("kim"), b"jpeg").unwrap();
        assert_eq!(
            fs::read(Contact::image_path(vault.root(), &slug("kim"))).unwrap(),
            b"jpeg"
        );
        assert!(vault.save_contact_image(&slug("nope"), b"jpeg").is_err());
        assert!(Contact::lone_images(vault.root()).unwrap().is_empty());
        let lone = Contact::image_path(vault.root(), &slug("nope"));
        fs::write(&lone, "").unwrap();
        fs::write(vault.root().join("contacts/kim.sync-conflict-1.jpg"), "").unwrap();
        assert_eq!(Contact::lone_images(vault.root()).unwrap(), [lone]);
    }
}
