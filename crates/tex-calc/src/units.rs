use std::f64::consts::PI;

use crate::Locale;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dim {
    Length,
    Mass,
    Area,
    Volume,
    Temperature,
    Speed,
    Time,
    Data,
    Angle,
}

/// A unit: `value * factor + offset` is the value in the base unit of its dimension
/// (m, kg, m², m³, K, m/s, s, byte, rad). `names` are folded (lowercase, no accents).
#[derive(Debug, PartialEq)]
pub struct Unit {
    pub symbol: &'static str,
    pub symbol_en: &'static str,
    pub plural_pt: &'static str,
    pub dim: Dim,
    pub factor: f64,
    pub offset: f64,
    pub names: &'static [&'static str],
}

impl Unit {
    pub fn to_base(&self, value: f64) -> f64 {
        value * self.factor + self.offset
    }

    pub fn from_base(&self, base: f64) -> f64 {
        (base - self.offset) / self.factor
    }

    pub fn symbol_for(&self, locale: Locale, singular: bool) -> &'static str {
        match locale {
            Locale::En => self.symbol_en,
            Locale::Pt if singular => self.symbol,
            Locale::Pt => self.plural_pt,
        }
    }
}

const fn u(symbol: &'static str, dim: Dim, factor: f64, names: &'static [&'static str]) -> Unit {
    Unit {
        symbol,
        symbol_en: symbol,
        plural_pt: symbol,
        dim,
        factor,
        offset: 0.0,
        names,
    }
}

const fn w(
    symbol: &'static str,
    plural_pt: &'static str,
    symbol_en: &'static str,
    dim: Dim,
    factor: f64,
    names: &'static [&'static str],
) -> Unit {
    Unit {
        symbol,
        symbol_en,
        plural_pt,
        dim,
        factor,
        offset: 0.0,
        names,
    }
}

const fn temperature(
    symbol: &'static str,
    factor: f64,
    offset: f64,
    names: &'static [&'static str],
) -> Unit {
    Unit {
        symbol,
        symbol_en: symbol,
        plural_pt: symbol,
        dim: Dim::Temperature,
        factor,
        offset,
        names,
    }
}

const INCH: f64 = 0.0254;

pub static UNITS: &[Unit] = &[
    u(
        "mm",
        Dim::Length,
        0.001,
        &[
            "mm",
            "milimetro",
            "milimetros",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
    ),
    u(
        "cm",
        Dim::Length,
        0.01,
        &[
            "cm",
            "centimetro",
            "centimetros",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
    ),
    u(
        "m",
        Dim::Length,
        1.0,
        &["m", "metro", "metros", "meter", "meters", "metre", "metres"],
    ),
    u(
        "km",
        Dim::Length,
        1000.0,
        &[
            "km",
            "quilometro",
            "quilometros",
            "kilometer",
            "kilometers",
            "kilometre",
            "kilometres",
        ],
    ),
    u(
        "in",
        Dim::Length,
        INCH,
        &["in", "inch", "inches", "pol", "polegada", "polegadas"],
    ),
    u(
        "ft",
        Dim::Length,
        0.3048,
        &["ft", "foot", "feet", "pe", "pes"],
    ),
    u(
        "yd",
        Dim::Length,
        0.9144,
        &["yd", "yard", "yards", "jarda", "jardas"],
    ),
    u(
        "mi",
        Dim::Length,
        1609.344,
        &["mi", "mile", "miles", "milha", "milhas"],
    ),
    u(
        "nmi",
        Dim::Length,
        1852.0,
        &[
            "nmi",
            "milha nautica",
            "milhas nauticas",
            "nautical mile",
            "nautical miles",
        ],
    ),
    u("px", Dim::Length, INCH / 96.0, &["px", "pixel", "pixels"]),
    u(
        "pt",
        Dim::Length,
        INCH / 72.0,
        &["pt", "point", "points", "ponto", "pontos"],
    ),
    u("rem", Dim::Length, INCH / 6.0, &["rem"]),
    u(
        "mg",
        Dim::Mass,
        1e-6,
        &["mg", "miligrama", "miligramas", "milligram", "milligrams"],
    ),
    u(
        "g",
        Dim::Mass,
        0.001,
        &["g", "grama", "gramas", "gram", "grams"],
    ),
    u(
        "kg",
        Dim::Mass,
        1.0,
        &[
            "kg",
            "quilo",
            "quilos",
            "quilograma",
            "quilogramas",
            "kilo",
            "kilos",
            "kilogram",
            "kilograms",
        ],
    ),
    u(
        "t",
        Dim::Mass,
        1000.0,
        &[
            "t",
            "tonelada",
            "toneladas",
            "tonne",
            "tonnes",
            "ton",
            "tons",
        ],
    ),
    u(
        "oz",
        Dim::Mass,
        0.028349523125,
        &["oz", "onca", "oncas", "ounce", "ounces"],
    ),
    u(
        "lb",
        Dim::Mass,
        0.45359237,
        &["lb", "lbs", "libra", "libras", "pound", "pounds"],
    ),
    u("st", Dim::Mass, 6.35029318, &["st", "stone", "stones"]),
    u("mm²", Dim::Area, 1e-6, &["mm2"]),
    u("cm²", Dim::Area, 1e-4, &["cm2"]),
    u(
        "m²",
        Dim::Area,
        1.0,
        &[
            "m2",
            "metro quadrado",
            "metros quadrados",
            "square meter",
            "square meters",
        ],
    ),
    u("km²", Dim::Area, 1e6, &["km2"]),
    u("ha", Dim::Area, 1e4, &["ha", "hectare", "hectares"]),
    u("acre", Dim::Area, 4046.8564224, &["acre", "acres", "ac"]),
    u("ft²", Dim::Area, 0.09290304, &["ft2", "sqft"]),
    u("in²", Dim::Area, 0.00064516, &["in2"]),
    u(
        "ml",
        Dim::Volume,
        1e-6,
        &[
            "ml",
            "mililitro",
            "mililitros",
            "milliliter",
            "milliliters",
            "millilitre",
            "millilitres",
        ],
    ),
    u("cl", Dim::Volume, 1e-5, &["cl"]),
    u(
        "l",
        Dim::Volume,
        1e-3,
        &["l", "litro", "litros", "liter", "liters", "litre", "litres"],
    ),
    u("m³", Dim::Volume, 1.0, &["m3"]),
    u("cm³", Dim::Volume, 1e-6, &["cm3", "cc"]),
    u(
        "tsp",
        Dim::Volume,
        4.92892159375e-6,
        &["tsp", "teaspoon", "teaspoons"],
    ),
    u(
        "tbsp",
        Dim::Volume,
        1.478676478125e-5,
        &["tbsp", "tablespoon", "tablespoons"],
    ),
    u(
        "cup",
        Dim::Volume,
        2.365882365e-4,
        &["cup", "cups", "xicara", "xicaras"],
    ),
    u(
        "fl oz",
        Dim::Volume,
        2.95735295625e-5,
        &["fl oz", "floz", "fluid ounce", "fluid ounces"],
    ),
    u("pint", Dim::Volume, 4.73176473e-4, &["pint", "pints"]),
    u("qt", Dim::Volume, 9.46352946e-4, &["qt", "quart", "quarts"]),
    u(
        "gal",
        Dim::Volume,
        3.785411784e-3,
        &["gal", "gallon", "gallons", "galao", "galoes"],
    ),
    temperature("°C", 1.0, 273.15, &["°c", "c", "celsius"]),
    temperature(
        "°F",
        5.0 / 9.0,
        459.67 * 5.0 / 9.0,
        &["°f", "f", "fahrenheit"],
    ),
    temperature("K", 1.0, 0.0, &["k", "kelvin", "kelvins"]),
    u("m/s", Dim::Speed, 1.0, &["m/s"]),
    u("km/h", Dim::Speed, 1.0 / 3.6, &["km/h", "kmh", "kph"]),
    u("mph", Dim::Speed, 0.44704, &["mph"]),
    u(
        "kn",
        Dim::Speed,
        1852.0 / 3600.0,
        &["kn", "kt", "knot", "knots", "nos"],
    ),
    u(
        "ms",
        Dim::Time,
        0.001,
        &[
            "ms",
            "milissegundo",
            "milissegundos",
            "millisecond",
            "milliseconds",
        ],
    ),
    u(
        "s",
        Dim::Time,
        1.0,
        &[
            "s", "sec", "secs", "seg", "segundo", "segundos", "second", "seconds",
        ],
    ),
    u(
        "min",
        Dim::Time,
        60.0,
        &["min", "mins", "minuto", "minutos", "minute", "minutes"],
    ),
    u(
        "h",
        Dim::Time,
        3600.0,
        &["h", "hr", "hrs", "hora", "horas", "hour", "hours"],
    ),
    u(
        "d",
        Dim::Time,
        86400.0,
        &["d", "dia", "dias", "day", "days"],
    ),
    w(
        "sem",
        "sem",
        "wk",
        Dim::Time,
        604800.0,
        &["sem", "semana", "semanas", "wk", "wks", "week", "weeks"],
    ),
    w(
        "mês",
        "meses",
        "mo",
        Dim::Time,
        2629746.0,
        &["mes", "meses", "mo", "month", "months"],
    ),
    w(
        "ano",
        "anos",
        "yr",
        Dim::Time,
        31556952.0,
        &["ano", "anos", "yr", "yrs", "year", "years"],
    ),
    u("bit", Dim::Data, 0.125, &["bit", "bits"]),
    u(
        "kbit",
        Dim::Data,
        125.0,
        &["kbit", "kbits", "kilobit", "kilobits"],
    ),
    u(
        "Mbit",
        Dim::Data,
        1.25e5,
        &["mbit", "mbits", "megabit", "megabits"],
    ),
    u(
        "Gbit",
        Dim::Data,
        1.25e8,
        &["gbit", "gbits", "gigabit", "gigabits"],
    ),
    u(
        "Tbit",
        Dim::Data,
        1.25e11,
        &["tbit", "tbits", "terabit", "terabits"],
    ),
    u("B", Dim::Data, 1.0, &["byte", "bytes"]),
    u("KB", Dim::Data, 1e3, &["kb", "kilobyte", "kilobytes"]),
    u("MB", Dim::Data, 1e6, &["mb", "megabyte", "megabytes"]),
    u("GB", Dim::Data, 1e9, &["gb", "gigabyte", "gigabytes"]),
    u("TB", Dim::Data, 1e12, &["tb", "terabyte", "terabytes"]),
    u("KiB", Dim::Data, 1024.0, &["kib", "kibibyte", "kibibytes"]),
    u(
        "MiB",
        Dim::Data,
        1048576.0,
        &["mib", "mebibyte", "mebibytes"],
    ),
    u(
        "GiB",
        Dim::Data,
        1073741824.0,
        &["gib", "gibibyte", "gibibytes"],
    ),
    u(
        "TiB",
        Dim::Data,
        1099511627776.0,
        &["tib", "tebibyte", "tebibytes"],
    ),
    u(
        "°",
        Dim::Angle,
        PI / 180.0,
        &["°", "deg", "degree", "degrees", "grau", "graus"],
    ),
    u(
        "rad",
        Dim::Angle,
        1.0,
        &["rad", "radian", "radians", "radiano", "radianos"],
    ),
];

const EXACT: &[(&str, &str)] = &[
    ("b", "bit"),
    ("B", "B"),
    ("Kb", "kbit"),
    ("Mb", "Mbit"),
    ("Gb", "Gbit"),
    ("Tb", "Tbit"),
];

/// Finds a unit by what the user typed: the case-sensitive data symbols first (`b` is a bit,
/// `B` a byte, `Mb` a megabit), then any folded name (`mb` and `MB` are megabytes).
pub fn lookup(text: &str) -> Option<&'static Unit> {
    if let Some((_, symbol)) = EXACT.iter().find(|(name, _)| *name == text) {
        return UNITS.iter().find(|unit| unit.symbol == *symbol);
    }
    let folded = tex_index::fold(text);
    UNITS
        .iter()
        .find(|unit| unit.names.contains(&folded.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(text: &str) -> Option<&'static str> {
        lookup(text).map(|unit| unit.symbol)
    }

    #[test]
    fn data_symbols_keep_their_case() {
        assert_eq!(symbol("b"), Some("bit"));
        assert_eq!(symbol("B"), Some("B"));
        assert_eq!(symbol("Mb"), Some("Mbit"));
        assert_eq!(symbol("mb"), Some("MB"));
        assert_eq!(symbol("MB"), Some("MB"));
        assert_eq!(symbol("KiB"), Some("KiB"));
    }

    #[test]
    fn names_are_found_without_case_or_accents() {
        assert_eq!(symbol("pés"), Some("ft"));
        assert_eq!(symbol("Polegadas"), Some("in"));
        assert_eq!(symbol("fl oz"), Some("fl oz"));
        assert_eq!(symbol("km/h"), Some("km/h"));
        assert_eq!(symbol("°C"), Some("°C"));
        assert_eq!(symbol("Mês"), Some("mês"));
        assert_eq!(symbol("xyz"), None);
        assert_eq!(symbol("em"), None);
    }

    #[test]
    fn no_name_belongs_to_two_units() {
        let mut seen = std::collections::HashSet::new();
        for unit in UNITS {
            for name in unit.names {
                assert!(seen.insert(*name), "{name} is used twice");
                assert_eq!(tex_index::fold(name), *name, "{name} is not folded");
            }
        }
    }

    #[test]
    fn temperature_goes_through_kelvin() {
        let celsius = lookup("c").unwrap();
        let fahrenheit = lookup("f").unwrap();
        let value = fahrenheit.from_base(celsius.to_base(100.0));
        assert!((value - 212.0).abs() < 1e-9);
    }

    #[test]
    fn portuguese_plural_only_for_named_units() {
        let year = lookup("ano").unwrap();
        assert_eq!(year.symbol_for(Locale::Pt, true), "ano");
        assert_eq!(year.symbol_for(Locale::Pt, false), "anos");
        assert_eq!(year.symbol_for(Locale::En, false), "yr");
        assert_eq!(lookup("km").unwrap().symbol_for(Locale::Pt, false), "km");
    }
}
