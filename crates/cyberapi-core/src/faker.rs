//! R3: Postman-compatible `$random*` Faker dynamic variables and Chinese data
//! template generators (`cnName`, `cnPhone`, `cnIdCard`).
//!
//! Pure, deterministic from [`crate::template::Draws`] (and `inputs.now` for
//! relative dates), with built-in compact wordlists and zero external crates so
//! desktop, web (WASM) and bridge behave identically.

use crate::Timestamp;
use crate::template::{Draws, Inputs};

/// Logical category of a Postman dynamic variable, shown in autocomplete hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DynCategory {
    Common,
    Text,
    Internet,
    Names,
    Profession,
    Location,
    Images,
    Finance,
    Business,
    Database,
    Dates,
    Domains,
    Files,
    Commerce,
    Grammar,
    Lorem,
}

impl DynCategory {
    /// Suffix used under `hints.dyn_<key>` in the locale tables.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Common => "common",
            Self::Text => "text",
            Self::Internet => "internet",
            Self::Names => "names",
            Self::Profession => "profession",
            Self::Location => "location",
            Self::Images => "images",
            Self::Finance => "finance",
            Self::Business => "business",
            Self::Database => "database",
            Self::Dates => "dates",
            Self::Domains => "domains",
            Self::Files => "files",
            Self::Commerce => "commerce",
            Self::Grammar => "grammar",
            Self::Lorem => "lorem",
        }
    }
}

/// Metadata for one `$...` dynamic variable offered in template hints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DynamicVar {
    pub name: &'static str,
    pub category: DynCategory,
    pub example: &'static str,
}

const fn dyn_var(name: &'static str, category: DynCategory, example: &'static str) -> DynamicVar {
    DynamicVar {
        name,
        category,
        example,
    }
}

/// Complete list of Postman Faker-style dynamic variables offered by hints.
pub const DYNAMIC_VARS: &[DynamicVar] = &[
    // Common
    dyn_var("$guid", DynCategory::Common, "550e8400-e29b-41d4-a716-446655440000"),
    dyn_var("$timestamp", DynCategory::Common, "1700000000"),
    dyn_var("$isoTimestamp", DynCategory::Common, "2026-11-22T08:30:00Z"),
    dyn_var("$randomInt", DynCategory::Common, "0–1000"),
    dyn_var("$randomAlphaNumeric", DynCategory::Common, "a"),
    dyn_var("$randomString", DynCategory::Common, "A7bX9qP2mK4nL8vR"),
    dyn_var("$randomBoolean", DynCategory::Common, "true / false"),
    dyn_var("$randomFloat", DynCategory::Common, "428.1930"),
    // Text & colors
    dyn_var("$randomHexColor", DynCategory::Text, "#3a7bd5"),
    dyn_var("$randomAbbreviation", DynCategory::Text, "HTTP"),
    dyn_var("$randomColor", DynCategory::Text, "azure"),
    // Internet & IP
    dyn_var("$randomIP", DynCategory::Internet, "192.168.14.88"),
    dyn_var("$randomIPV6", DynCategory::Internet, "2001:0db8:85a3::8a2e"),
    dyn_var("$randomMACAddress", DynCategory::Internet, "3c:5a:b4:12:9f:0e"),
    dyn_var("$randomPassword", DynCategory::Internet, "k9XmP2qL7vR4wN8"),
    dyn_var("$randomLocale", DynCategory::Internet, "en_US"),
    dyn_var("$randomUserAgent", DynCategory::Internet, "Mozilla/5.0 …"),
    dyn_var("$randomProtocol", DynCategory::Internet, "https"),
    dyn_var("$randomSemver", DynCategory::Internet, "2.14.7"),
    // Names
    dyn_var("$randomFirstName", DynCategory::Names, "Alice"),
    dyn_var("$randomLastName", DynCategory::Names, "Chen"),
    dyn_var("$randomFullName", DynCategory::Names, "Alice Chen"),
    dyn_var("$randomNamePrefix", DynCategory::Names, "Dr."),
    dyn_var("$randomNameSuffix", DynCategory::Names, "Jr."),
    // Profession
    dyn_var("$randomJobArea", DynCategory::Profession, "Infrastructure"),
    dyn_var("$randomJobDescriptor", DynCategory::Profession, "Principal"),
    dyn_var(
        "$randomJobTitle",
        DynCategory::Profession,
        "Principal Solutions Architect",
    ),
    dyn_var("$randomJobType", DynCategory::Profession, "Architect"),
    // Phone, address & location
    dyn_var("$randomPhoneNumber", DynCategory::Location, "415-892-3104"),
    dyn_var("$randomPhoneNumberExt", DynCategory::Location, "1-415-892-3104 x421"),
    dyn_var("$randomCity", DynCategory::Location, "Seattle"),
    dyn_var("$randomStreetName", DynCategory::Location, "Maple Avenue"),
    dyn_var("$randomStreetAddress", DynCategory::Location, "742 Maple Avenue"),
    dyn_var("$randomCountry", DynCategory::Location, "Canada"),
    dyn_var("$randomCountryCode", DynCategory::Location, "CA"),
    dyn_var("$randomLatitude", DynCategory::Location, "37.7749"),
    dyn_var("$randomLongitude", DynCategory::Location, "-122.4194"),
    // Images
    dyn_var(
        "$randomAvatarImage",
        DynCategory::Images,
        "https://cdn.example.com/images/avatar/42.jpg",
    ),
    dyn_var(
        "$randomImageUrl",
        DynCategory::Images,
        "https://cdn.example.com/images/photo/42.jpg",
    ),
    dyn_var(
        "$randomAbstractImage",
        DynCategory::Images,
        "https://cdn.example.com/images/abstract/42.jpg",
    ),
    dyn_var(
        "$randomAnimalsImage",
        DynCategory::Images,
        "https://cdn.example.com/images/animals/42.jpg",
    ),
    dyn_var(
        "$randomBusinessImage",
        DynCategory::Images,
        "https://cdn.example.com/images/business/42.jpg",
    ),
    dyn_var(
        "$randomCatsImage",
        DynCategory::Images,
        "https://cdn.example.com/images/cats/42.jpg",
    ),
    dyn_var(
        "$randomCityImage",
        DynCategory::Images,
        "https://cdn.example.com/images/city/42.jpg",
    ),
    dyn_var(
        "$randomFoodImage",
        DynCategory::Images,
        "https://cdn.example.com/images/food/42.jpg",
    ),
    dyn_var(
        "$randomNightlifeImage",
        DynCategory::Images,
        "https://cdn.example.com/images/nightlife/42.jpg",
    ),
    dyn_var(
        "$randomFashionImage",
        DynCategory::Images,
        "https://cdn.example.com/images/fashion/42.jpg",
    ),
    dyn_var(
        "$randomPeopleImage",
        DynCategory::Images,
        "https://cdn.example.com/images/people/42.jpg",
    ),
    dyn_var(
        "$randomNatureImage",
        DynCategory::Images,
        "https://cdn.example.com/images/nature/42.jpg",
    ),
    dyn_var(
        "$randomSportsImage",
        DynCategory::Images,
        "https://cdn.example.com/images/sports/42.jpg",
    ),
    dyn_var(
        "$randomTransportImage",
        DynCategory::Images,
        "https://cdn.example.com/images/transport/42.jpg",
    ),
    dyn_var("$randomImageDataUri", DynCategory::Images, "data:image/svg+xml;utf8,…"),
    // Finance
    dyn_var("$randomBankAccount", DynCategory::Finance, "48291037"),
    dyn_var("$randomBankAccountName", DynCategory::Finance, "Checking Account"),
    dyn_var("$randomCreditCardMask", DynCategory::Finance, "4242"),
    dyn_var("$randomBankAccountBic", DynCategory::Finance, "BANKUS42XXX"),
    dyn_var("$randomBankAccountIban", DynCategory::Finance, "DE89370400440532013000"),
    dyn_var("$randomTransactionType", DynCategory::Finance, "payment"),
    dyn_var("$randomCurrencyCode", DynCategory::Finance, "USD"),
    dyn_var("$randomCurrencyName", DynCategory::Finance, "US Dollar"),
    dyn_var("$randomCurrencySymbol", DynCategory::Finance, "$"),
    dyn_var(
        "$randomBitcoin",
        DynCategory::Finance,
        "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
    ),
    // Business
    dyn_var("$randomCompanyName", DynCategory::Business, "Northwind Technologies"),
    dyn_var("$randomCompanySuffix", DynCategory::Business, "Inc."),
    dyn_var("$randomBs", DynCategory::Business, "orchestrate scalable platforms"),
    dyn_var("$randomBsAdjective", DynCategory::Business, "scalable"),
    dyn_var("$randomBsBuzz", DynCategory::Business, "orchestrate"),
    dyn_var("$randomBsNoun", DynCategory::Business, "platforms"),
    dyn_var(
        "$randomCatchPhrase",
        DynCategory::Business,
        "Distributed zero-defect interface",
    ),
    dyn_var("$randomCatchPhraseAdjective", DynCategory::Business, "Distributed"),
    dyn_var("$randomCatchPhraseDescriptor", DynCategory::Business, "zero-defect"),
    dyn_var("$randomCatchPhraseNoun", DynCategory::Business, "interface"),
    // Database
    dyn_var("$randomDatabaseColumn", DynCategory::Database, "created_at"),
    dyn_var("$randomDatabaseType", DynCategory::Database, "varchar"),
    dyn_var("$randomDatabaseCollation", DynCategory::Database, "utf8mb4_unicode_ci"),
    dyn_var("$randomDatabaseEngine", DynCategory::Database, "InnoDB"),
    // Dates
    dyn_var("$randomDateFuture", DynCategory::Dates, "2027-04-12T10:24:00Z"),
    dyn_var("$randomDatePast", DynCategory::Dates, "2026-03-19T14:05:12Z"),
    dyn_var("$randomDateRecent", DynCategory::Dates, "2026-11-22T02:11:45Z"),
    dyn_var("$randomWeekday", DynCategory::Dates, "Wednesday"),
    dyn_var("$randomMonth", DynCategory::Dates, "November"),
    // Domains, emails & usernames
    dyn_var("$randomDomainName", DynCategory::Domains, "aurora-cloud.dev"),
    dyn_var("$randomDomainSuffix", DynCategory::Domains, "dev"),
    dyn_var("$randomDomainWord", DynCategory::Domains, "aurora"),
    dyn_var("$randomEmail", DynCategory::Domains, "alice.chen42@aurora-cloud.dev"),
    dyn_var("$randomExampleEmail", DynCategory::Domains, "alice.chen42@example.com"),
    dyn_var("$randomUserName", DynCategory::Domains, "alice.chen42"),
    dyn_var("$randomUrl", DynCategory::Domains, "https://aurora-cloud.dev"),
    // Files & directories
    dyn_var("$randomFileName", DynCategory::Files, "report.json"),
    dyn_var("$randomFileType", DynCategory::Files, "application"),
    dyn_var("$randomFileExt", DynCategory::Files, "json"),
    dyn_var("$randomCommonFileName", DynCategory::Files, "invoice.pdf"),
    dyn_var("$randomCommonFileType", DynCategory::Files, "text"),
    dyn_var("$randomCommonFileExt", DynCategory::Files, "pdf"),
    dyn_var("$randomFilePath", DynCategory::Files, "/var/data/report.json"),
    dyn_var("$randomDirectoryPath", DynCategory::Files, "/var/data"),
    dyn_var("$randomMimeType", DynCategory::Files, "application/json"),
    // Commerce / Stores
    dyn_var("$randomPrice", DynCategory::Commerce, "149.99"),
    dyn_var("$randomProduct", DynCategory::Commerce, "Keyboard"),
    dyn_var("$randomProductAdjective", DynCategory::Commerce, "Ergonomic"),
    dyn_var("$randomProductMaterial", DynCategory::Commerce, "Aluminum"),
    dyn_var(
        "$randomProductName",
        DynCategory::Commerce,
        "Ergonomic Aluminum Keyboard",
    ),
    dyn_var("$randomDepartment", DynCategory::Commerce, "Electronics"),
    // Grammar
    dyn_var("$randomNoun", DynCategory::Grammar, "protocol"),
    dyn_var("$randomVerb", DynCategory::Grammar, "stream"),
    dyn_var("$randomIngverb", DynCategory::Grammar, "streaming"),
    dyn_var("$randomAdjective", DynCategory::Grammar, "resilient"),
    dyn_var("$randomWord", DynCategory::Grammar, "gateway"),
    dyn_var("$randomWords", DynCategory::Grammar, "gateway protocol stream"),
    dyn_var(
        "$randomPhrase",
        DynCategory::Grammar,
        "Try to stream the resilient protocol",
    ),
    // Lorem
    dyn_var("$randomLoremWord", DynCategory::Lorem, "tempor"),
    dyn_var("$randomLoremWords", DynCategory::Lorem, "lorem ipsum dolor"),
    dyn_var(
        "$randomLoremSentence",
        DynCategory::Lorem,
        "Lorem ipsum dolor sit amet.",
    ),
    dyn_var(
        "$randomLoremSentences",
        DynCategory::Lorem,
        "Lorem ipsum dolor. Sit amet.",
    ),
    dyn_var("$randomLoremLines", DynCategory::Lorem, "Lorem ipsum.\nDolor sit amet."),
    dyn_var(
        "$randomLoremParagraph",
        DynCategory::Lorem,
        "Lorem ipsum dolor sit amet…",
    ),
    dyn_var(
        "$randomLoremParagraphs",
        DynCategory::Lorem,
        "Lorem ipsum…\n\nConsectetur…",
    ),
    dyn_var("$randomLoremText", DynCategory::Lorem, "Lorem ipsum dolor sit amet…"),
    dyn_var("$randomLoremSlug", DynCategory::Lorem, "lorem-ipsum-dolor"),
];

/// Whether `name` is one of the Faker `$random*` dynamic variables evaluated
/// by this module (the 6 core `$guid` / `$timestamp` / `$randomInt` / etc. are
/// handled directly in `crate::template`).
pub(crate) fn is_known(name: &str) -> bool {
    matches!(
        name,
        "$randomBoolean"
            | "$randomFloat"
            | "$randomHexColor"
            | "$randomAbbreviation"
            | "$randomColor"
            | "$randomIP"
            | "$randomIPV6"
            | "$randomMACAddress"
            | "$randomPassword"
            | "$randomLocale"
            | "$randomUserAgent"
            | "$randomProtocol"
            | "$randomSemver"
            | "$randomFirstName"
            | "$randomLastName"
            | "$randomFullName"
            | "$randomNamePrefix"
            | "$randomNameSuffix"
            | "$randomJobArea"
            | "$randomJobDescriptor"
            | "$randomJobTitle"
            | "$randomJobType"
            | "$randomPhoneNumber"
            | "$randomPhoneNumberExt"
            | "$randomCity"
            | "$randomStreetName"
            | "$randomStreetAddress"
            | "$randomCountry"
            | "$randomCountryCode"
            | "$randomLatitude"
            | "$randomLongitude"
            | "$randomAvatarImage"
            | "$randomImageUrl"
            | "$randomAbstractImage"
            | "$randomAnimalsImage"
            | "$randomBusinessImage"
            | "$randomCatsImage"
            | "$randomCityImage"
            | "$randomFoodImage"
            | "$randomNightlifeImage"
            | "$randomFashionImage"
            | "$randomPeopleImage"
            | "$randomNatureImage"
            | "$randomSportsImage"
            | "$randomTransportImage"
            | "$randomImageDataUri"
            | "$randomBankAccount"
            | "$randomBankAccountName"
            | "$randomCreditCardMask"
            | "$randomBankAccountBic"
            | "$randomBankAccountIban"
            | "$randomTransactionType"
            | "$randomCurrencyCode"
            | "$randomCurrencyName"
            | "$randomCurrencySymbol"
            | "$randomBitcoin"
            | "$randomCompanyName"
            | "$randomCompanySuffix"
            | "$randomBs"
            | "$randomBsAdjective"
            | "$randomBsBuzz"
            | "$randomBsNoun"
            | "$randomCatchPhrase"
            | "$randomCatchPhraseAdjective"
            | "$randomCatchPhraseDescriptor"
            | "$randomCatchPhraseNoun"
            | "$randomDatabaseColumn"
            | "$randomDatabaseType"
            | "$randomDatabaseCollation"
            | "$randomDatabaseEngine"
            | "$randomDateFuture"
            | "$randomDatePast"
            | "$randomDateRecent"
            | "$randomWeekday"
            | "$randomMonth"
            | "$randomDomainName"
            | "$randomDomainSuffix"
            | "$randomDomainWord"
            | "$randomEmail"
            | "$randomExampleEmail"
            | "$randomUserName"
            | "$randomUrl"
            | "$randomFileName"
            | "$randomFileType"
            | "$randomFileExt"
            | "$randomCommonFileName"
            | "$randomCommonFileType"
            | "$randomCommonFileExt"
            | "$randomFilePath"
            | "$randomDirectoryPath"
            | "$randomMimeType"
            | "$randomPrice"
            | "$randomProduct"
            | "$randomProductAdjective"
            | "$randomProductMaterial"
            | "$randomProductName"
            | "$randomDepartment"
            | "$randomNoun"
            | "$randomVerb"
            | "$randomIngverb"
            | "$randomAdjective"
            | "$randomWord"
            | "$randomWords"
            | "$randomPhrase"
            | "$randomLoremWord"
            | "$randomLoremWords"
            | "$randomLoremSentence"
            | "$randomLoremSentences"
            | "$randomLoremLines"
            | "$randomLoremParagraph"
            | "$randomLoremParagraphs"
            | "$randomLoremText"
            | "$randomLoremSlug"
    )
}

/// Evaluates a Faker `$random*` dynamic variable from `inputs.draws` and
/// `inputs.now`; returns `None` when `name` is unknown.
pub(crate) fn eval(name: &str, inputs: &Inputs) -> Option<String> {
    let draws = &inputs.draws;
    if let Some(value) = eval_common_internet_location(name, draws) {
        return Some(value);
    }
    if let Some(value) = eval_images_finance_business(name, draws) {
        return Some(value);
    }
    eval_data_files_text(name, inputs)
}

fn eval_common_internet_location(name: &str, draws: &Draws) -> Option<String> {
    let out = match name {
        "$randomBoolean" => if draws.next().is_multiple_of(2) {
            "true"
        } else {
            "false"
        }
        .into(),
        "$randomFloat" => format!("{}.{:04}", draws.between(0, 999), draws.between(0, 9999)),
        "$randomHexColor" => format!("#{:06x}", draws.next() & 0x00ff_ffff),
        "$randomAbbreviation" => pick(ABBREVIATIONS, draws).into(),
        "$randomColor" => pick(COLORS, draws).into(),
        "$randomIP" => format!(
            "{}.{}.{}.{}",
            draws.between(1, 254),
            draws.between(0, 255),
            draws.between(0, 255),
            draws.between(1, 254),
        ),
        "$randomIPV6" => (0..8)
            .map(|_| format!("{:04x}", draws.next() & 0xffff))
            .collect::<Vec<_>>()
            .join(":"),
        "$randomMACAddress" => (0..6)
            .map(|_| format!("{:02x}", draws.next() & 0xff))
            .collect::<Vec<_>>()
            .join(":"),
        "$randomPassword" => random_chars(
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789",
            15,
            draws,
        ),
        "$randomLocale" => pick(LOCALES, draws).into(),
        "$randomUserAgent" => pick(USER_AGENTS, draws).into(),
        "$randomProtocol" => if draws.next().is_multiple_of(2) {
            "https"
        } else {
            "http"
        }
        .into(),
        "$randomSemver" => format!(
            "{}.{}.{}",
            draws.between(0, 9),
            draws.between(0, 20),
            draws.between(0, 50)
        ),
        "$randomFirstName" => pick(FIRST_NAMES, draws).into(),
        "$randomLastName" => pick(LAST_NAMES, draws).into(),
        "$randomFullName" => format!("{} {}", pick(FIRST_NAMES, draws), pick(LAST_NAMES, draws)),
        "$randomNamePrefix" => pick(NAME_PREFIXES, draws).into(),
        "$randomNameSuffix" => pick(NAME_SUFFIXES, draws).into(),
        "$randomJobArea" => pick(JOB_AREAS, draws).into(),
        "$randomJobDescriptor" => pick(JOB_DESCRIPTORS, draws).into(),
        "$randomJobTitle" => format!(
            "{} {} {}",
            pick(JOB_DESCRIPTORS, draws),
            pick(JOB_AREAS, draws),
            pick(JOB_TYPES, draws)
        ),
        "$randomJobType" => pick(JOB_TYPES, draws).into(),
        "$randomPhoneNumber" => format!(
            "{:03}-{:03}-{:04}",
            draws.between(200, 999),
            draws.between(200, 999),
            draws.between(1000, 9999)
        ),
        "$randomPhoneNumberExt" => format!(
            "1-{:03}-{:03}-{:04} x{:03}",
            draws.between(200, 999),
            draws.between(200, 999),
            draws.between(1000, 9999),
            draws.between(100, 999)
        ),
        "$randomCity" => pick(CITIES, draws).into(),
        "$randomStreetName" => format!("{} {}", pick(STREET_NAMES, draws), pick(STREET_SUFFIXES, draws)),
        "$randomStreetAddress" => format!(
            "{} {} {}",
            draws.between(1, 9999),
            pick(STREET_NAMES, draws),
            pick(STREET_SUFFIXES, draws)
        ),
        "$randomCountry" => pick_pair(COUNTRIES, draws).0.into(),
        "$randomCountryCode" => pick_pair(COUNTRIES, draws).1.into(),
        "$randomLatitude" => signed_decimal(draws.between(-899_999, 899_999)),
        "$randomLongitude" => signed_decimal(draws.between(-1_799_999, 1_799_999)),
        _ => return None,
    };
    Some(out)
}

fn eval_images_finance_business(name: &str, draws: &Draws) -> Option<String> {
    let image_url = |kind: &str| format!("https://cdn.example.com/images/{kind}/{}.jpg", draws.between(1, 9999));
    let out = match name {
        "$randomAvatarImage" => image_url("avatar"),
        "$randomImageUrl" => image_url("photo"),
        "$randomAbstractImage" => image_url("abstract"),
        "$randomAnimalsImage" => image_url("animals"),
        "$randomBusinessImage" => image_url("business"),
        "$randomCatsImage" => image_url("cats"),
        "$randomCityImage" => image_url("city"),
        "$randomFoodImage" => image_url("food"),
        "$randomNightlifeImage" => image_url("nightlife"),
        "$randomFashionImage" => image_url("fashion"),
        "$randomPeopleImage" => image_url("people"),
        "$randomNatureImage" => image_url("nature"),
        "$randomSportsImage" => image_url("sports"),
        "$randomTransportImage" => image_url("transport"),
        "$randomImageDataUri" => format!(
            "data:image/svg+xml;utf8,%3Csvg%20xmlns='http://www.w3.org/2000/svg'%20width='64'%20height='64'%3E%3Crect%20width='64'%20height='64'%20fill='%23{:06x}'/%3E%3C/svg%3E",
            draws.next() & 0x00ff_ffff
        ),
        "$randomBankAccount" => format!("{:08}", draws.between(10_000_000, 99_999_999)),
        "$randomBankAccountName" => pick(BANK_ACCOUNTS, draws).into(),
        "$randomCreditCardMask" => format!("{:04}", draws.between(0, 9999)),
        "$randomBankAccountBic" => format!("BANK{}{:02}XXX", pick_pair(COUNTRIES, draws).1, draws.between(10, 99)),
        "$randomBankAccountIban" => format!(
            "{}{:02}{:04}{:012}",
            pick_pair(COUNTRIES, draws).1,
            draws.between(10, 98),
            draws.between(1000, 9999),
            draws.between(100_000_000_000, 999_999_999_999)
        ),
        "$randomTransactionType" => pick(TRANSACTION_TYPES, draws).into(),
        "$randomCurrencyCode" => CURRENCIES[(draws.next() as usize) % CURRENCIES.len()].0.into(),
        "$randomCurrencyName" => CURRENCIES[(draws.next() as usize) % CURRENCIES.len()].1.into(),
        "$randomCurrencySymbol" => CURRENCIES[(draws.next() as usize) % CURRENCIES.len()].2.into(),
        "$randomBitcoin" => format!(
            "1{}",
            random_chars(b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz", 33, draws)
        ),
        "$randomCompanyName" => format!(
            "{} {}",
            pick(DOMAIN_WORDS, draws).to_ascii_uppercase_first(),
            pick(COMPANY_SUFFIXES, draws)
        ),
        "$randomCompanySuffix" => pick(COMPANY_SUFFIXES, draws).into(),
        "$randomBs" => format!(
            "{} {} {}",
            pick(BS_BUZZ, draws),
            pick(BS_ADJECTIVES, draws),
            pick(BS_NOUNS, draws)
        ),
        "$randomBsAdjective" => pick(BS_ADJECTIVES, draws).into(),
        "$randomBsBuzz" => pick(BS_BUZZ, draws).into(),
        "$randomBsNoun" => pick(BS_NOUNS, draws).into(),
        "$randomCatchPhrase" => format!(
            "{} {} {}",
            pick(CP_ADJECTIVES, draws),
            pick(CP_DESCRIPTORS, draws),
            pick(CP_NOUNS, draws)
        ),
        "$randomCatchPhraseAdjective" => pick(CP_ADJECTIVES, draws).into(),
        "$randomCatchPhraseDescriptor" => pick(CP_DESCRIPTORS, draws).into(),
        "$randomCatchPhraseNoun" => pick(CP_NOUNS, draws).into(),
        _ => return None,
    };
    Some(out)
}

fn eval_data_files_text(name: &str, inputs: &Inputs) -> Option<String> {
    let draws = &inputs.draws;
    let domain = || format!("{}.{}", pick(DOMAIN_WORDS, draws), pick(DOMAIN_SUFFIXES, draws));
    let username = || {
        format!(
            "{}.{}{}",
            pick(FIRST_NAMES, draws).to_ascii_lowercase(),
            pick(LAST_NAMES, draws).to_ascii_lowercase(),
            draws.between(1, 99)
        )
    };
    let sentence = |words: usize| -> String {
        let joined = (0..words)
            .map(|_| pick(LOREM_WORDS, draws))
            .collect::<Vec<_>>()
            .join(" ");
        format!("{}.", joined.to_ascii_uppercase_first())
    };
    let paragraph = |sentences: usize| (0..sentences).map(|_| sentence(6)).collect::<Vec<_>>().join(" ");

    let out = match name {
        "$randomDatabaseColumn" => pick(DB_COLUMNS, draws).into(),
        "$randomDatabaseType" => pick(DB_TYPES, draws).into(),
        "$randomDatabaseCollation" => pick(DB_COLLATIONS, draws).into(),
        "$randomDatabaseEngine" => pick(DB_ENGINES, draws).into(),
        "$randomDateFuture" => {
            Timestamp::from_unix(inputs.now.saturating_add(draws.between(60, 365 * 86_400))).to_rfc3339()
        }
        "$randomDatePast" => {
            Timestamp::from_unix(inputs.now.saturating_sub(draws.between(60, 365 * 86_400))).to_rfc3339()
        }
        "$randomDateRecent" => Timestamp::from_unix(inputs.now.saturating_sub(draws.between(1, 86_400))).to_rfc3339(),
        "$randomWeekday" => pick(WEEKDAYS, draws).into(),
        "$randomMonth" => pick(MONTHS, draws).into(),
        "$randomDomainName" => domain(),
        "$randomDomainSuffix" => pick(DOMAIN_SUFFIXES, draws).into(),
        "$randomDomainWord" => pick(DOMAIN_WORDS, draws).into(),
        "$randomEmail" => format!("{}@{}", username(), domain()),
        "$randomExampleEmail" => format!("{}@example.com", username()),
        "$randomUserName" => username(),
        "$randomUrl" => format!("https://{}", domain()),
        "$randomFileName" => format!("{}.{}", pick(FILE_STEMS, draws), pick(FILE_EXTS, draws)),
        "$randomFileType" => pick(FILE_TYPES, draws).into(),
        "$randomFileExt" => pick(FILE_EXTS, draws).into(),
        "$randomCommonFileName" => format!("{}.{}", pick(FILE_STEMS, draws), pick(COMMON_FILE_EXTS, draws)),
        "$randomCommonFileType" => pick(COMMON_FILE_TYPES, draws).into(),
        "$randomCommonFileExt" => pick(COMMON_FILE_EXTS, draws).into(),
        "$randomFilePath" => format!(
            "{}/{}.{}",
            pick(DIRECTORIES, draws),
            pick(FILE_STEMS, draws),
            pick(FILE_EXTS, draws)
        ),
        "$randomDirectoryPath" => pick(DIRECTORIES, draws).into(),
        "$randomMimeType" => pick(MIME_TYPES, draws).into(),
        "$randomPrice" => format!("{}.{:02}", draws.between(1, 999), draws.between(0, 99)),
        "$randomProduct" => pick(PRODUCTS, draws).into(),
        "$randomProductAdjective" => pick(PRODUCT_ADJECTIVES, draws).into(),
        "$randomProductMaterial" => pick(PRODUCT_MATERIALS, draws).into(),
        "$randomProductName" => format!(
            "{} {} {}",
            pick(PRODUCT_ADJECTIVES, draws),
            pick(PRODUCT_MATERIALS, draws),
            pick(PRODUCTS, draws)
        ),
        "$randomDepartment" => pick(DEPARTMENTS, draws).into(),
        "$randomNoun" => pick(GRAMMAR_NOUNS, draws).into(),
        "$randomVerb" => pick(GRAMMAR_VERBS, draws).into(),
        "$randomIngverb" => pick(GRAMMAR_INGVERBS, draws).into(),
        "$randomAdjective" => pick(GRAMMAR_ADJECTIVES, draws).into(),
        "$randomWord" => pick(LOREM_WORDS, draws).into(),
        "$randomWords" => (0..3).map(|_| pick(LOREM_WORDS, draws)).collect::<Vec<_>>().join(" "),
        "$randomPhrase" => format!(
            "Try to {} the {} {}",
            pick(GRAMMAR_VERBS, draws),
            pick(GRAMMAR_ADJECTIVES, draws),
            pick(GRAMMAR_NOUNS, draws)
        ),
        "$randomLoremWord" => pick(LOREM_WORDS, draws).into(),
        "$randomLoremWords" => (0..4).map(|_| pick(LOREM_WORDS, draws)).collect::<Vec<_>>().join(" "),
        "$randomLoremSentence" => sentence(6),
        "$randomLoremSentences" => (0..3).map(|_| sentence(5)).collect::<Vec<_>>().join(" "),
        "$randomLoremLines" => (0..3).map(|_| sentence(5)).collect::<Vec<_>>().join("\n"),
        "$randomLoremParagraph" => paragraph(3),
        "$randomLoremParagraphs" => (0..2).map(|_| paragraph(3)).collect::<Vec<_>>().join("\n\n"),
        "$randomLoremText" => paragraph(4),
        "$randomLoremSlug" => (0..3).map(|_| pick(LOREM_WORDS, draws)).collect::<Vec<_>>().join("-"),
        _ => return None,
    };
    Some(out)
}

/// Generates a random Chinese full name (common surname + 1 or 2 given-name characters).
pub(crate) fn cn_name(draws: &Draws) -> String {
    let surname = pick(CN_SURNAMES, draws);
    let len = if draws.next().is_multiple_of(3) { 1 } else { 2 };
    let mut name = String::from(surname);
    for _ in 0..len {
        name.push_str(pick(CN_GIVEN_CHARS, draws));
    }
    name
}

/// Generates a random mainland China 11-digit mobile phone number with a valid
/// carrier prefix.
pub(crate) fn cn_phone(draws: &Draws) -> String {
    let prefix = pick(CN_PHONE_PREFIXES, draws);
    format!("{prefix}{:08}", draws.between(0, 99_999_999))
}

/// Generates a random 18-digit mainland China resident ID card number
/// (GB 11643-1999) with a valid administrative division prefix, calendar birth
/// date, sequence code, and ISO 7064:1983.MOD 11-2 checksum digit.
pub(crate) fn cn_id_card(draws: &Draws) -> String {
    const WEIGHTS: [u32; 17] = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
    const CHECK_TABLE: [u8; 11] = *b"10X98765432";

    let area = pick(CN_AREA_CODES, draws);
    let year = draws.between(1960, 2005);
    let month = draws.between(1, 12);
    let max_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    };
    let day = draws.between(1, max_day);
    let seq = draws.between(1, 999);
    let body = format!("{area}{year:04}{month:02}{day:02}{seq:03}");
    let sum: u32 = body
        .bytes()
        .zip(WEIGHTS)
        .map(|(byte, weight)| u32::from(byte.saturating_sub(b'0')) * weight)
        .sum();
    let check = char::from(CHECK_TABLE[(sum % 11) as usize]);
    format!("{body}{check}")
}

fn pick(list: &[&'static str], draws: &Draws) -> &'static str {
    list[(draws.next() as usize) % list.len()]
}

fn pick_pair(list: &[(&'static str, &'static str)], draws: &Draws) -> (&'static str, &'static str) {
    list[(draws.next() as usize) % list.len()]
}

fn random_chars(alphabet: &[u8], len: usize, draws: &Draws) -> String {
    (0..len)
        .map(|_| char::from(alphabet[(draws.next() as usize) % alphabet.len()]))
        .collect()
}

fn signed_decimal(scaled_10k: i64) -> String {
    let sign = if scaled_10k < 0 { "-" } else { "" };
    let abs = scaled_10k.unsigned_abs();
    format!("{sign}{}.{:04}", abs / 10_000, abs % 10_000)
}

trait AsciiTitleCase {
    fn to_ascii_uppercase_first(&self) -> String;
}

impl AsciiTitleCase for str {
    fn to_ascii_uppercase_first(&self) -> String {
        let mut chars = self.chars();
        match chars.next() {
            Some(first) => format!("{}{}", first.to_ascii_uppercase(), chars.as_str()),
            None => String::new(),
        }
    }
}

const ABBREVIATIONS: &[&str] = &[
    "HTTP", "TCP", "SQL", "API", "JSON", "XML", "CSS", "RAM", "CPU", "GPU", "SSD", "USB", "DNS", "TLS", "SSH", "FTP",
    "SMTP", "IMAP", "REST", "GRPC", "SDK", "CLI", "IDE", "GUI",
];

const COLORS: &[&str] = &[
    "azure",
    "crimson",
    "emerald",
    "indigo",
    "amber",
    "violet",
    "cobalt",
    "coral",
    "teal",
    "silver",
    "gold",
    "ivory",
    "magenta",
    "olive",
    "orchid",
    "plum",
    "salmon",
    "sienna",
    "slate",
    "tan",
    "turquoise",
    "wheat",
    "maroon",
    "navy",
];

const LOCALES: &[&str] = &[
    "en_US", "en_GB", "zh_CN", "zh_TW", "ja_JP", "ko_KR", "fr_FR", "de_DE", "es_ES", "pt_BR", "it_IT", "ru_RU",
    "nl_NL", "sv_SE", "pl_PL", "tr_TR", "ar_SA", "hi_IN", "id_ID", "vi_VN",
];

const USER_AGENTS: &[&str] = &[
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36",
    "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Safari/605.1.15",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1",
    "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
];

const FIRST_NAMES: &[&str] = &[
    "Alice", "Bob", "Carol", "David", "Elena", "Frank", "Grace", "Henry", "Iris", "Jack", "Kate", "Liam", "Maya",
    "Noah", "Olivia", "Paul", "Quinn", "Ryan", "Sofia", "Tom", "Uma", "Victor", "Wendy", "Xander", "Yara", "Zane",
    "Daniel", "Emma", "Lucas", "Chloe",
];

const LAST_NAMES: &[&str] = &[
    "Smith", "Johnson", "Williams", "Brown", "Jones", "Garcia", "Miller", "Davis", "Wilson", "Moore", "Taylor",
    "Anderson", "Thomas", "Jackson", "White", "Harris", "Martin", "Thompson", "Lee", "Walker", "Hall", "Allen",
    "Young", "King", "Wright", "Lopez", "Hill", "Scott", "Green", "Chen",
];

const NAME_PREFIXES: &[&str] = &["Mr.", "Mrs.", "Ms.", "Dr.", "Prof.", "Mx."];
const NAME_SUFFIXES: &[&str] = &["Jr.", "Sr.", "II", "III", "PhD", "MD"];

const JOB_DESCRIPTORS: &[&str] = &[
    "Lead",
    "Senior",
    "Principal",
    "Global",
    "Dynamic",
    "District",
    "Regional",
    "National",
    "Direct",
    "Forward",
    "Central",
    "Chief",
    "Internal",
    "Human",
    "Corporate",
    "Product",
];

const JOB_AREAS: &[&str] = &[
    "Infrastructure",
    "Security",
    "Operations",
    "Optimization",
    "Research",
    "Integration",
    "Metrics",
    "Mobility",
    "Quality",
    "Accounts",
    "Data",
    "Identity",
    "Assurance",
    "Configuration",
    "Branding",
    "Directives",
];

const JOB_TYPES: &[&str] = &[
    "Architect",
    "Engineer",
    "Coordinator",
    "Supervisor",
    "Specialist",
    "Director",
    "Designer",
    "Developer",
    "Analyst",
    "Manager",
    "Planner",
    "Officer",
    "Consultant",
    "Strategist",
    "Administrator",
    "Lead",
];

const CITIES: &[&str] = &[
    "Seattle",
    "Austin",
    "Boston",
    "Denver",
    "Chicago",
    "Portland",
    "Toronto",
    "Vancouver",
    "London",
    "Bristol",
    "Dublin",
    "Berlin",
    "Munich",
    "Zurich",
    "Paris",
    "Lyon",
    "Amsterdam",
    "Stockholm",
    "Tokyo",
    "Osaka",
    "Singapore",
    "Sydney",
    "Melbourne",
    "Shanghai",
    "Shenzhen",
];

const STREET_NAMES: &[&str] = &[
    "Maple", "Oak", "Cedar", "Pine", "Elm", "Birch", "Willow", "Sunset", "Highland", "Meadow", "River", "Lakeview",
    "Park", "Spring", "Harbor", "Summit",
];

const STREET_SUFFIXES: &[&str] = &[
    "Street",
    "Avenue",
    "Boulevard",
    "Road",
    "Lane",
    "Drive",
    "Court",
    "Way",
    "Place",
    "Terrace",
    "Circle",
    "Trail",
];

const COUNTRIES: &[(&str, &str)] = &[
    ("United States", "US"),
    ("Canada", "CA"),
    ("United Kingdom", "GB"),
    ("Germany", "DE"),
    ("France", "FR"),
    ("Japan", "JP"),
    ("China", "CN"),
    ("Australia", "AU"),
    ("Netherlands", "NL"),
    ("Sweden", "SE"),
    ("Switzerland", "CH"),
    ("Singapore", "SG"),
    ("South Korea", "KR"),
    ("Spain", "ES"),
    ("Italy", "IT"),
    ("Brazil", "BR"),
    ("India", "IN"),
    ("Ireland", "IE"),
    ("Norway", "NO"),
    ("Finland", "FI"),
    ("Denmark", "DK"),
    ("Belgium", "BE"),
    ("Austria", "AT"),
    ("Poland", "PL"),
];

const BANK_ACCOUNTS: &[&str] = &[
    "Checking Account",
    "Savings Account",
    "Money Market Account",
    "Investment Account",
    "Credit Card Account",
    "Auto Loan Account",
    "Home Loan Account",
    "Personal Loan Account",
];

const TRANSACTION_TYPES: &[&str] = &["deposit", "withdrawal", "payment", "invoice"];

const CURRENCIES: &[(&str, &str, &str)] = &[
    ("USD", "US Dollar", "$"),
    ("EUR", "Euro", "€"),
    ("GBP", "Pound Sterling", "£"),
    ("JPY", "Yen", "¥"),
    ("CNY", "Yuan Renminbi", "¥"),
    ("CHF", "Swiss Franc", "CHF"),
    ("CAD", "Canadian Dollar", "CA$"),
    ("AUD", "Australian Dollar", "A$"),
    ("SGD", "Singapore Dollar", "S$"),
    ("HKD", "Hong Kong Dollar", "HK$"),
    ("KRW", "Won", "₩"),
    ("INR", "Indian Rupee", "₹"),
];

const COMPANY_SUFFIXES: &[&str] = &[
    "Inc.",
    "LLC",
    "Group",
    "Corp.",
    "Ltd.",
    "and Sons",
    "Partners",
    "Technologies",
];

const BS_BUZZ: &[&str] = &[
    "orchestrate",
    "streamline",
    "optimize",
    "accelerate",
    "empower",
    "scale",
    "unify",
    "synthesize",
    "architect",
    "redefine",
    "deploy",
    "transform",
    "benchmark",
    "automate",
];

const BS_ADJECTIVES: &[&str] = &[
    "scalable",
    "resilient",
    "distributed",
    "real-time",
    "serverless",
    "turnkey",
    "enterprise",
    "seamless",
    "frictionless",
    "modular",
    "robust",
    "extensible",
    "proactive",
    "cloud-native",
];

const BS_NOUNS: &[&str] = &[
    "platforms",
    "pipelines",
    "workflows",
    "APIs",
    "schemas",
    "interfaces",
    "networks",
    "protocols",
    "metrics",
    "services",
    "infrastructures",
    "clusters",
    "gateways",
    "ecosystems",
];

const CP_ADJECTIVES: &[&str] = &[
    "Adaptive",
    "Automated",
    "Distributed",
    "Balanced",
    "Centralized",
    "Configurable",
    "Digitized",
    "Expanded",
    "Focused",
    "Integrated",
    "Managed",
    "Networked",
    "Open-source",
    "Programmable",
    "Reactive",
    "Seamless",
];

const CP_DESCRIPTORS: &[&str] = &[
    "zero-defect",
    "fault-tolerant",
    "bandwidth-monitored",
    "high-availability",
    "context-sensitive",
    "mission-critical",
    "multi-tiered",
    "non-volatile",
    "object-oriented",
    "scalable",
    "systematic",
    "uniform",
];

const CP_NOUNS: &[&str] = &[
    "architecture",
    "capacity",
    "database",
    "emulator",
    "firmware",
    "framework",
    "hierarchy",
    "interface",
    "middleware",
    "monitoring",
    "orchestration",
    "protocol",
    "repository",
    "solution",
];

const DB_COLUMNS: &[&str] = &[
    "id",
    "created_at",
    "updated_at",
    "status",
    "email",
    "username",
    "token",
    "amount",
    "title",
    "category",
    "version",
    "comment",
    "avatar",
    "password",
];

const DB_TYPES: &[&str] = &[
    "varchar",
    "bigint",
    "int",
    "text",
    "boolean",
    "timestamp",
    "uuid",
    "jsonb",
    "decimal",
    "float",
    "blob",
    "date",
];

const DB_COLLATIONS: &[&str] = &[
    "utf8mb4_unicode_ci",
    "utf8mb4_general_ci",
    "utf8_general_ci",
    "latin1_swedish_ci",
    "ascii_general_ci",
    "C.UTF-8",
];

const DB_ENGINES: &[&str] = &["InnoDB", "MyISAM", "MEMORY", "CSV", "ARCHIVE", "BLACKHOLE"];

const WEEKDAYS: &[&str] = &[
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

const MONTHS: &[&str] = &[
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

const DOMAIN_WORDS: &[&str] = &[
    "aurora", "beacon", "cobalt", "delta", "ember", "falcon", "glacier", "harbor", "iris", "jade", "kestrel",
    "lantern", "meridian", "nexus", "orbit", "prism", "quartz", "ripple", "summit", "timber", "umbra", "vertex",
    "zephyr", "nova",
];

const DOMAIN_SUFFIXES: &[&str] = &["com", "net", "org", "io", "dev", "co", "app", "tech", "info", "biz"];

const FILE_STEMS: &[&str] = &[
    "report", "invoice", "backup", "config", "schema", "payload", "metrics", "export", "summary", "profile", "ledger",
    "manifest", "archive", "notes",
];

const FILE_EXTS: &[&str] = &[
    "json", "yaml", "csv", "txt", "pdf", "png", "svg", "xml", "html", "log", "zip", "md", "ts", "rs",
];

const FILE_TYPES: &[&str] = &[
    "application",
    "text",
    "image",
    "audio",
    "video",
    "multipart",
    "font",
    "model",
];

const COMMON_FILE_EXTS: &[&str] = &["pdf", "txt", "csv", "json", "png", "jpg", "html", "gif"];
const COMMON_FILE_TYPES: &[&str] = &["application", "text", "image", "audio", "video"];

const DIRECTORIES: &[&str] = &[
    "/var/data",
    "/opt/app/cache",
    "/srv/uploads",
    "/etc/cyberapi",
    "/home/user/exports",
    "/tmp/work",
    "/usr/local/share",
    "/var/log/api",
];

const MIME_TYPES: &[&str] = &[
    "application/json",
    "application/xml",
    "application/pdf",
    "application/octet-stream",
    "text/plain",
    "text/csv",
    "text/html",
    "image/png",
    "image/jpeg",
    "image/svg+xml",
    "multipart/form-data",
];

const PRODUCTS: &[&str] = &[
    "Keyboard",
    "Monitor",
    "Headphones",
    "Mouse",
    "Desk",
    "Chair",
    "Backpack",
    "Watch",
    "Bottle",
    "Lamp",
    "Speaker",
    "Router",
    "Notebook",
    "Camera",
    "Tripod",
    "Microphone",
];

const PRODUCT_ADJECTIVES: &[&str] = &[
    "Ergonomic",
    "Sleek",
    "Compact",
    "Durable",
    "Refined",
    "Practical",
    "Lightweight",
    "Modern",
    "Rustic",
    "Handcrafted",
    "Intelligent",
    "Generic",
    "Awesome",
    "Licensed",
];

const PRODUCT_MATERIALS: &[&str] = &[
    "Aluminum", "Steel", "Wooden", "Carbon", "Ceramic", "Glass", "Leather", "Cotton", "Granite", "Titanium", "Plastic",
    "Bronze",
];

const DEPARTMENTS: &[&str] = &[
    "Electronics",
    "Books",
    "Home",
    "Garden",
    "Office",
    "Sports",
    "Outdoors",
    "Music",
    "Health",
    "Games",
    "Clothing",
    "Tools",
    "Grocery",
    "Automotive",
];

const GRAMMAR_NOUNS: &[&str] = &[
    "protocol",
    "gateway",
    "firewall",
    "bus",
    "bandwidth",
    "capacitor",
    "driver",
    "feed",
    "hard drive",
    "interface",
    "matrix",
    "microchip",
    "monitor",
    "panel",
    "pixel",
    "port",
    "program",
    "sensor",
    "system",
    "transmitter",
];

const GRAMMAR_VERBS: &[&str] = &[
    "bypass",
    "calculate",
    "compress",
    "connect",
    "copy",
    "generate",
    "hack",
    "index",
    "input",
    "navigate",
    "override",
    "parse",
    "program",
    "quantify",
    "reboot",
    "stream",
    "synthesize",
    "transmit",
];

const GRAMMAR_INGVERBS: &[&str] = &[
    "bypassing",
    "calculating",
    "compressing",
    "connecting",
    "copying",
    "generating",
    "indexing",
    "navigating",
    "overriding",
    "parsing",
    "quantifying",
    "rebooting",
    "streaming",
    "synthesizing",
    "transmitting",
];

const GRAMMAR_ADJECTIVES: &[&str] = &[
    "auxiliary",
    "back-end",
    "bluetooth",
    "cross-platform",
    "digital",
    "haptic",
    "mobile",
    "multi-byte",
    "neural",
    "online",
    "open-source",
    "optical",
    "primary",
    "redundant",
    "resilient",
    "solid state",
    "virtual",
    "wireless",
];

const LOREM_WORDS: &[&str] = &[
    "lorem",
    "ipsum",
    "dolor",
    "sit",
    "amet",
    "consectetur",
    "adipiscing",
    "elit",
    "sed",
    "do",
    "eiusmod",
    "tempor",
    "incididunt",
    "ut",
    "labore",
    "et",
    "dolore",
    "magna",
    "aliqua",
    "enim",
    "ad",
    "minim",
    "veniam",
    "quis",
    "nostrud",
    "exercitation",
    "ullamco",
    "laboris",
    "nisi",
    "aliquip",
    "ex",
    "ea",
    "commodo",
    "consequat",
    "duis",
    "aute",
    "irure",
    "in",
    "reprehenderit",
    "voluptate",
    "velit",
    "esse",
    "cillum",
    "fugiat",
    "nulla",
    "pariatur",
];

const CN_SURNAMES: &[&str] = &[
    "王", "李", "张", "刘", "陈", "杨", "黄", "赵", "吴", "周", "徐", "孙", "马", "朱", "胡", "郭", "何", "林", "高",
    "罗", "郑", "梁", "谢", "宋", "唐", "许", "韩", "冯", "邓", "曹", "彭", "曾", "肖", "田", "董", "袁", "潘", "于",
    "蒋", "蔡", "余", "杜", "叶", "程", "苏", "魏", "吕", "丁", "任", "沈",
];

const CN_GIVEN_CHARS: &[&str] = &[
    "伟", "芳", "娜", "秀", "敏", "静", "丽", "强", "磊", "军", "洋", "勇", "艳", "杰", "娟", "涛", "明", "超", "平",
    "刚", "桂", "英", "玉", "兰", "霞", "峰", "辉", "宇", "浩", "博", "子", "涵", "轩", "梓", "欣", "怡", "晨", "阳",
    "泽", "瑞", "思", "雨", "梦", "佳", "俊", "凯", "鹏", "飞", "翔", "帆", "嘉", "文", "华", "建", "国", "安", "康",
    "宁", "乐", "慧", "婷", "雪", "梅", "竹", "松", "柏", "林", "森", "海", "江", "河", "山", "川", "云", "风", "星",
    "月", "光", "宁", "悦",
];

const CN_PHONE_PREFIXES: &[&str] = &[
    "130", "131", "132", "133", "134", "135", "136", "137", "138", "139", "150", "151", "152", "153", "155", "156",
    "157", "158", "159", "166", "176", "177", "178", "180", "181", "182", "183", "184", "185", "186", "187", "188",
    "189", "198", "199",
];

const CN_AREA_CODES: &[&str] = &[
    "110101", "110105", "110108", "120101", "130102", "140105", "150102", "210102", "220102", "230102", "310101",
    "310104", "310115", "320102", "320505", "330102", "330106", "330203", "340102", "350102", "350203", "360102",
    "370102", "370202", "410102", "420102", "430102", "440103", "440106", "440303", "440305", "450102", "460105",
    "500103", "510104", "520102", "530102", "610102",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{NoFiles, has_dynamic, is_known_dollar_var, render};
    use std::collections::HashSet;

    fn inputs(seed: u64) -> Inputs {
        Inputs {
            now: 1_700_000_000,
            draws: Draws::new(seed),
            ..Default::default()
        }
    }

    #[test]
    fn every_dynamic_var_is_recognized_marked_dynamic_and_evaluated() {
        let mut seen = HashSet::new();
        for var in DYNAMIC_VARS {
            assert!(seen.insert(var.name), "duplicate dynamic var {}", var.name);
            assert!(is_known_dollar_var(var.name), "not known dollar var: {}", var.name);
            let expr = format!("{{{{{}}}}}", var.name);
            assert!(has_dynamic(&expr), "not marked dynamic: {expr}");
            let a = render(&expr, &inputs(11), &NoFiles("")).expect("render a");
            let b = render(&expr, &inputs(29), &NoFiles("")).expect("render b");
            assert!(!a.is_empty(), "{} evaluated to empty", var.name);
            assert!(!b.is_empty(), "{} evaluated to empty", var.name);
            assert!(!a.contains("{{"), "{} left unresolved braces: {a}", var.name);
        }
        assert_ne!(
            render("{{$randomEmail}}", &inputs(11), &NoFiles("")).expect("a"),
            render("{{$randomEmail}}", &inputs(29), &NoFiles("")).expect("b"),
        );
        let cn = render("{{cnName()}}|{{cnPhone()}}|{{cnIdCard()}}", &inputs(7), &NoFiles("")).expect("cn");
        let parts: Vec<&str> = cn.split('|').collect();
        assert_eq!(parts.len(), 3);
        assert!((2..=3).contains(&parts[0].chars().count()));
        assert_eq!(parts[1].len(), 11);
        assert_eq!(parts[2].len(), 18);
    }

    #[test]
    fn dynamic_vars_format_invariants_hold() {
        let inp = inputs(77);
        let email = render("{{$randomEmail}}", &inp, &NoFiles("")).expect("email");
        assert!(email.contains('@') && email.contains('.'), "email: {email}");

        let ip = render("{{$randomIP}}", &inp, &NoFiles("")).expect("ip");
        let octets: Vec<u8> = ip.split('.').map(|s| s.parse().expect("u8")).collect();
        assert_eq!(octets.len(), 4);

        let ipv6 = render("{{$randomIPV6}}", &inp, &NoFiles("")).expect("ipv6");
        assert_eq!(ipv6.split(':').count(), 8);

        let mac = render("{{$randomMACAddress}}", &inp, &NoFiles("")).expect("mac");
        assert_eq!(mac.split(':').count(), 6);

        let hex = render("{{$randomHexColor}}", &inp, &NoFiles("")).expect("hex");
        assert!(hex.starts_with('#') && hex.len() == 7, "hex: {hex}");

        let price = render("{{$randomPrice}}", &inp, &NoFiles("")).expect("price");
        assert!(price.parse::<f64>().is_ok(), "price: {price}");

        let lat: f64 = render("{{$randomLatitude}}", &inp, &NoFiles(""))
            .expect("lat")
            .parse()
            .expect("f64");
        assert!((-90.0..=90.0).contains(&lat));

        let lon: f64 = render("{{$randomLongitude}}", &inp, &NoFiles(""))
            .expect("lon")
            .parse()
            .expect("f64");
        assert!((-180.0..=180.0).contains(&lon));

        let future = Timestamp::parse_lenient(&render("{{$randomDateFuture}}", &inp, &NoFiles("")).expect("future"));
        let past = Timestamp::parse_lenient(&render("{{$randomDatePast}}", &inp, &NoFiles("")).expect("past"));
        let recent = Timestamp::parse_lenient(&render("{{$randomDateRecent}}", &inp, &NoFiles("")).expect("recent"));
        assert!(future.as_second() > inp.now);
        assert!(past.as_second() < inp.now);
        assert!(recent.as_second() < inp.now && recent.as_second() >= inp.now - 86_400);
    }

    #[test]
    fn chinese_generators_produce_valid_names_phones_and_id_cards() {
        const WEIGHTS: [u32; 17] = [7, 9, 10, 5, 8, 4, 2, 1, 6, 3, 7, 9, 10, 5, 8, 4, 2];
        const CHECK_TABLE: [u8; 11] = *b"10X98765432";

        let draws = Draws::new(42);
        for _ in 0..200 {
            let name = cn_name(&draws);
            let chars = name.chars().count();
            assert!((2..=3).contains(&chars), "unexpected cnName len: {name}");

            let phone = cn_phone(&draws);
            assert_eq!(phone.len(), 11);
            assert!(phone.bytes().all(|b| b.is_ascii_digit()));
            assert!(CN_PHONE_PREFIXES.contains(&&phone[..3]), "prefix: {phone}");

            let id = cn_id_card(&draws);
            assert_eq!(id.len(), 18, "id: {id}");
            assert!(id[..17].bytes().all(|b| b.is_ascii_digit()));
            assert!(CN_AREA_CODES.contains(&&id[..6]));
            let year: i32 = id[6..10].parse().expect("year");
            let month: u32 = id[10..12].parse().expect("month");
            let day: u32 = id[12..14].parse().expect("day");
            assert!((1960..=2005).contains(&year));
            assert!((1..=12).contains(&month));
            let expected_max_day = match month {
                1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                4 | 6 | 9 | 11 => 30,
                _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
                _ => 28,
            };
            assert!((1..=expected_max_day).contains(&day), "bad date in {id}");
            let sum: u32 = id[..17]
                .bytes()
                .zip(WEIGHTS)
                .map(|(b, w)| u32::from(b - b'0') * w)
                .sum();
            let expected_check = char::from(CHECK_TABLE[(sum % 11) as usize]);
            assert_eq!(id.as_bytes()[17] as char, expected_check, "checksum in {id}");
        }
    }
}
