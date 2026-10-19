use std::collections::HashMap;
use std::sync::OnceLock;

use jiff::tz::{self, Offset, TimeZone};

#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub tz: TimeZone,
}

const REGIONS: &[&str] = &[
    "Africa/",
    "America/",
    "Antarctica/",
    "Asia/",
    "Atlantic/",
    "Australia/",
    "Europe/",
    "Indian/",
    "Pacific/",
];

const ALIASES: &[(&str, &str, &str)] = &[
    ("toquio", "Tóquio", "Asia/Tokyo"),
    ("japao", "Japão", "Asia/Tokyo"),
    ("japan", "Japan", "Asia/Tokyo"),
    ("sao paulo", "São Paulo", "America/Sao_Paulo"),
    ("brasil", "Brasil", "America/Sao_Paulo"),
    ("brazil", "Brazil", "America/Sao_Paulo"),
    ("rio de janeiro", "Rio de Janeiro", "America/Sao_Paulo"),
    ("rio", "Rio de Janeiro", "America/Sao_Paulo"),
    ("brasilia", "Brasília", "America/Sao_Paulo"),
    ("belo horizonte", "Belo Horizonte", "America/Sao_Paulo"),
    ("curitiba", "Curitiba", "America/Sao_Paulo"),
    ("porto alegre", "Porto Alegre", "America/Sao_Paulo"),
    ("salvador", "Salvador", "America/Bahia"),
    ("belem", "Belém", "America/Belem"),
    ("cuiaba", "Cuiabá", "America/Cuiaba"),
    ("nova york", "Nova York", "America/New_York"),
    ("nova iorque", "Nova Iorque", "America/New_York"),
    ("nyc", "NYC", "America/New_York"),
    ("washington", "Washington", "America/New_York"),
    ("boston", "Boston", "America/New_York"),
    ("miami", "Miami", "America/New_York"),
    ("eua", "EUA", "America/New_York"),
    ("usa", "USA", "America/New_York"),
    ("estados unidos", "Estados Unidos", "America/New_York"),
    ("montreal", "Montreal", "America/Toronto"),
    ("canada", "Canadá", "America/Toronto"),
    ("san francisco", "San Francisco", "America/Los_Angeles"),
    ("sao francisco", "São Francisco", "America/Los_Angeles"),
    ("seattle", "Seattle", "America/Los_Angeles"),
    ("las vegas", "Las Vegas", "America/Los_Angeles"),
    ("dallas", "Dallas", "America/Chicago"),
    ("houston", "Houston", "America/Chicago"),
    ("austin", "Austin", "America/Chicago"),
    (
        "cidade do mexico",
        "Cidade do México",
        "America/Mexico_City",
    ),
    ("mexico", "México", "America/Mexico_City"),
    ("argentina", "Argentina", "America/Argentina/Buenos_Aires"),
    ("chile", "Chile", "America/Santiago"),
    ("peru", "Peru", "America/Lima"),
    ("bogota", "Bogotá", "America/Bogota"),
    ("colombia", "Colômbia", "America/Bogota"),
    ("montevideu", "Montevidéu", "America/Montevideo"),
    ("uruguai", "Uruguai", "America/Montevideo"),
    ("assuncao", "Assunção", "America/Asuncion"),
    ("paraguai", "Paraguai", "America/Asuncion"),
    ("londres", "Londres", "Europe/London"),
    ("inglaterra", "Inglaterra", "Europe/London"),
    ("england", "England", "Europe/London"),
    ("reino unido", "Reino Unido", "Europe/London"),
    ("uk", "UK", "Europe/London"),
    ("irlanda", "Irlanda", "Europe/Dublin"),
    ("lisboa", "Lisboa", "Europe/Lisbon"),
    ("portugal", "Portugal", "Europe/Lisbon"),
    ("porto", "Porto", "Europe/Lisbon"),
    ("madri", "Madri", "Europe/Madrid"),
    ("espanha", "Espanha", "Europe/Madrid"),
    ("spain", "Spain", "Europe/Madrid"),
    ("barcelona", "Barcelona", "Europe/Madrid"),
    ("franca", "França", "Europe/Paris"),
    ("france", "France", "Europe/Paris"),
    ("berlim", "Berlim", "Europe/Berlin"),
    ("alemanha", "Alemanha", "Europe/Berlin"),
    ("germany", "Germany", "Europe/Berlin"),
    ("munique", "Munique", "Europe/Berlin"),
    ("munich", "Munich", "Europe/Berlin"),
    ("frankfurt", "Frankfurt", "Europe/Berlin"),
    ("roma", "Roma", "Europe/Rome"),
    ("italia", "Itália", "Europe/Rome"),
    ("italy", "Italy", "Europe/Rome"),
    ("milao", "Milão", "Europe/Rome"),
    ("milan", "Milan", "Europe/Rome"),
    ("amsterda", "Amsterdã", "Europe/Amsterdam"),
    ("holanda", "Holanda", "Europe/Amsterdam"),
    ("netherlands", "Netherlands", "Europe/Amsterdam"),
    ("bruxelas", "Bruxelas", "Europe/Brussels"),
    ("zurique", "Zurique", "Europe/Zurich"),
    ("suica", "Suíça", "Europe/Zurich"),
    ("switzerland", "Switzerland", "Europe/Zurich"),
    ("genebra", "Genebra", "Europe/Zurich"),
    ("geneva", "Geneva", "Europe/Zurich"),
    ("viena", "Viena", "Europe/Vienna"),
    ("praga", "Praga", "Europe/Prague"),
    ("varsovia", "Varsóvia", "Europe/Warsaw"),
    ("atenas", "Atenas", "Europe/Athens"),
    ("estocolmo", "Estocolmo", "Europe/Stockholm"),
    ("copenhague", "Copenhague", "Europe/Copenhagen"),
    ("moscou", "Moscou", "Europe/Moscow"),
    ("istambul", "Istambul", "Europe/Istanbul"),
    ("turquia", "Turquia", "Europe/Istanbul"),
    ("emirados", "Emirados", "Asia/Dubai"),
    ("pequim", "Pequim", "Asia/Shanghai"),
    ("beijing", "Beijing", "Asia/Shanghai"),
    ("xangai", "Xangai", "Asia/Shanghai"),
    ("china", "China", "Asia/Shanghai"),
    ("singapura", "Singapura", "Asia/Singapore"),
    ("seul", "Seul", "Asia/Seoul"),
    ("coreia", "Coreia", "Asia/Seoul"),
    ("korea", "Korea", "Asia/Seoul"),
    ("india", "Índia", "Asia/Kolkata"),
    ("nova delhi", "Nova Délhi", "Asia/Kolkata"),
    ("new delhi", "New Delhi", "Asia/Kolkata"),
    ("delhi", "Delhi", "Asia/Kolkata"),
    ("mumbai", "Mumbai", "Asia/Kolkata"),
    ("bombaim", "Bombaim", "Asia/Kolkata"),
    ("bangalore", "Bangalore", "Asia/Kolkata"),
    ("tailandia", "Tailândia", "Asia/Bangkok"),
    ("jacarta", "Jacarta", "Asia/Jakarta"),
    ("filipinas", "Filipinas", "Asia/Manila"),
    ("tel aviv", "Tel Aviv", "Asia/Jerusalem"),
    ("israel", "Israel", "Asia/Jerusalem"),
    ("australia", "Austrália", "Australia/Sydney"),
    ("nova zelandia", "Nova Zelândia", "Pacific/Auckland"),
    ("havai", "Havaí", "Pacific/Honolulu"),
    ("hawaii", "Hawaii", "Pacific/Honolulu"),
    ("joanesburgo", "Joanesburgo", "Africa/Johannesburg"),
    ("egito", "Egito", "Africa/Cairo"),
    ("angola", "Angola", "Africa/Luanda"),
    ("mocambique", "Moçambique", "Africa/Maputo"),
    ("cabo verde", "Cabo Verde", "Atlantic/Cape_Verde"),
];

/// Folded city name → (IANA name, city as written in it), from the last part of every
/// regional IANA name; the shortest name wins (`America/Buenos_Aires` before
/// `America/Argentina/Buenos_Aires`).
fn index() -> &'static HashMap<String, (String, String)> {
    static INDEX: OnceLock<HashMap<String, (String, String)>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut names: Vec<String> = tz::db()
            .available()
            .map(|name| name.as_str().to_owned())
            .filter(|name| REGIONS.iter().any(|region| name.starts_with(region)))
            .collect();
        names.sort_by_key(|name| (name.matches('/').count(), name.clone()));
        let mut index = HashMap::new();
        for name in names {
            let Some((_, city)) = name.rsplit_once('/') else {
                continue;
            };
            let city = city.replace('_', " ");
            index
                .entry(tex_index::fold(&city))
                .or_insert_with(|| (name.clone(), city));
        }
        index
    })
}

/// The place a folded name stands for: an alias (Portuguese names, countries, cities without
/// a zone of their own), the city part of an IANA name (`tokyo`, `new york`), `utc` or `gmt`.
pub fn find(key: &str) -> Option<Place> {
    if matches!(key, "utc" | "gmt") {
        return Some(Place {
            name: "UTC".to_owned(),
            tz: TimeZone::UTC,
        });
    }
    let (iana, name) = ALIASES
        .iter()
        .find(|(alias, _, _)| *alias == key)
        .map(|(_, name, iana)| ((*iana).to_owned(), (*name).to_owned()))
        .or_else(|| index().get(key).cloned())?;
    let tz = tz::db().get(&iana).ok()?;
    Some(Place { name, tz })
}

/// A fixed `UTC±h[:mm]` place; `None` beyond 14 hours.
pub fn fixed(minutes: i32) -> Option<Place> {
    if minutes.abs() > 14 * 60 {
        return None;
    }
    let offset = Offset::from_seconds(minutes * 60).ok()?;
    let sign = if minutes < 0 { '-' } else { '+' };
    let (hours, rest) = (minutes.abs() / 60, minutes.abs() % 60);
    let name = if rest == 0 {
        format!("UTC{sign}{hours}")
    } else {
        format!("UTC{sign}{hours}:{rest:02}")
    };
    Some(Place {
        name,
        tz: TimeZone::fixed(offset),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iana(key: &str) -> Option<String> {
        find(key).and_then(|place| place.tz.iana_name().map(str::to_owned))
    }

    #[test]
    fn cities_come_from_the_database_and_the_aliases() {
        assert_eq!(find("tokyo").unwrap().name, "Tokyo");
        assert_eq!(iana("tokyo").as_deref(), Some("Asia/Tokyo"));
        assert_eq!(find("toquio").unwrap().name, "Tóquio");
        assert_eq!(find("sao paulo").unwrap().name, "São Paulo");
        assert_eq!(iana("new york").as_deref(), Some("America/New_York"));
        assert_eq!(iana("rio de janeiro").as_deref(), Some("America/Sao_Paulo"));
        assert_eq!(
            iana("buenos aires").as_deref(),
            Some("America/Buenos_Aires")
        );
        assert_eq!(find("marte"), None);
        assert_eq!(find("eastern"), None);
        assert_eq!(find("utc").unwrap().name, "UTC");
    }

    #[test]
    fn fixed_offsets() {
        assert_eq!(fixed(180).unwrap().name, "UTC+3");
        assert_eq!(fixed(-210).unwrap().name, "UTC-3:30");
        assert_eq!(fixed(0).unwrap().name, "UTC+0");
        assert_eq!(fixed(15 * 60), None);
    }

    #[test]
    fn every_alias_is_folded_unique_and_known() {
        let mut seen = std::collections::HashSet::new();
        for (key, _, iana) in ALIASES {
            assert!(seen.insert(*key), "{key} is used twice");
            assert_eq!(tex_index::fold(key), *key, "{key} is not folded");
            assert!(tz::db().get(iana).is_ok(), "{iana} is unknown");
        }
    }
}
