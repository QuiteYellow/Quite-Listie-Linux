//! List background gradients — ported from the Swift app's `BackgroundGradient.all`
//! (`extensions.swift:306-444`). Each gradient carries a light- and dark-mode pair of
//! hex stops; the UI resolves which pair to use from the active colour scheme. A list's
//! chosen background is persisted by id (see `ListBackground::gradient`) and rendered as
//! a top-leading → bottom-trailing linear gradient behind the list content, matching
//! Swift `ListView`'s `.background` (ListView.swift:666-676).

/// A named two-stop gradient with separate light/dark colour pairs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackgroundGradient {
    pub id: &'static str,
    pub name: &'static str,
    pub light_from: &'static str,
    pub light_to: &'static str,
    pub dark_from: &'static str,
    pub dark_to: &'static str,
}

impl BackgroundGradient {
    /// The from/to hex stops for the requested colour scheme.
    pub fn stops(&self, dark: bool) -> (&'static str, &'static str) {
        if dark {
            (self.dark_from, self.dark_to)
        } else {
            (self.light_from, self.light_to)
        }
    }
}

/// All gradients, grouped by mood (same order as the Swift source so ids line up).
pub const ALL: &[BackgroundGradient] = &[
    // Warm
    BackgroundGradient { id: "sunrise", name: "Sunrise", light_from: "#FFECD2", light_to: "#FCB69F", dark_from: "#4A2C1A", dark_to: "#8B3A2A" },
    BackgroundGradient { id: "peach-fuzz", name: "Peach Fuzz", light_from: "#FDD6BD", light_to: "#F9A8B8", dark_from: "#5C3429", dark_to: "#6E3044" },
    BackgroundGradient { id: "golden-hour", name: "Golden Hour", light_from: "#F6D365", light_to: "#FDA085", dark_from: "#5A4520", dark_to: "#6E3A2A" },
    BackgroundGradient { id: "rosewater", name: "Rosewater", light_from: "#FECFEF", light_to: "#FF989C", dark_from: "#4E2840", dark_to: "#6B2E30" },
    BackgroundGradient { id: "ember", name: "Ember", light_from: "#FF9A9E", light_to: "#FECFEF", dark_from: "#6B2E30", dark_to: "#4E2840" },
    BackgroundGradient { id: "coral-reef", name: "Coral Reef", light_from: "#F093FB", light_to: "#F5576C", dark_from: "#502855", dark_to: "#6E2233" },
    // Cool
    BackgroundGradient { id: "arctic", name: "Arctic", light_from: "#E0F7FA", light_to: "#B2EBF2", dark_from: "#0E3A40", dark_to: "#1A4E55" },
    BackgroundGradient { id: "deep-ocean", name: "Deep Ocean", light_from: "#A8EDEA", light_to: "#FED6E3", dark_from: "#1A4040", dark_to: "#4E2838" },
    BackgroundGradient { id: "moonrise", name: "Moonrise", light_from: "#C1DEFF", light_to: "#E8D5F5", dark_from: "#1A2E4A", dark_to: "#32254A" },
    BackgroundGradient { id: "pacific", name: "Pacific", light_from: "#667EEA", light_to: "#764BA2", dark_from: "#1E2755", dark_to: "#2D1A40" },
    BackgroundGradient { id: "frost", name: "Frost", light_from: "#D4FC79", light_to: "#96E6A1", dark_from: "#2A4020", dark_to: "#1E3A28" },
    BackgroundGradient { id: "northern-lights", name: "Northern Lights", light_from: "#43E97B", light_to: "#38F9D7", dark_from: "#0E3A1E", dark_to: "#0C3A38" },
    // Purple & Lavender
    BackgroundGradient { id: "wisteria", name: "Wisteria", light_from: "#C471F5", light_to: "#FA71CD", dark_from: "#381850", dark_to: "#501838" },
    BackgroundGradient { id: "amethyst", name: "Amethyst", light_from: "#DDD6F3", light_to: "#FAACA8", dark_from: "#2A2540", dark_to: "#4A2828" },
    BackgroundGradient { id: "grape-soda", name: "Grape Soda", light_from: "#9795F0", light_to: "#FBC8D4", dark_from: "#262445", dark_to: "#4A2838" },
    BackgroundGradient { id: "twilight", name: "Twilight", light_from: "#A18CD1", light_to: "#FBC2EB", dark_from: "#2A1845", dark_to: "#4E2845" },
    BackgroundGradient { id: "velvet", name: "Velvet", light_from: "#C33764", light_to: "#1D2671", dark_from: "#6B1A34", dark_to: "#0E1338" },
    // Nature
    BackgroundGradient { id: "sage-mist", name: "Sage Mist", light_from: "#C9D6C4", light_to: "#E8DFD0", dark_from: "#2A3828", dark_to: "#38322A" },
    BackgroundGradient { id: "forest-floor", name: "Forest Floor", light_from: "#56AB2F", light_to: "#A8E063", dark_from: "#1A3A0E", dark_to: "#2A4A18" },
    BackgroundGradient { id: "spring-meadow", name: "Spring Meadow", light_from: "#FBED96", light_to: "#ABECD6", dark_from: "#4A4220", dark_to: "#1E4038" },
    BackgroundGradient { id: "moss", name: "Moss", light_from: "#134E5E", light_to: "#71B280", dark_from: "#0A2830", dark_to: "#254030" },
    // Sunset & Sky
    BackgroundGradient { id: "california", name: "California", light_from: "#FF7E5F", light_to: "#FEB47B", dark_from: "#6B3028", dark_to: "#6E4828" },
    BackgroundGradient { id: "mango", name: "Mango", light_from: "#FFD89B", light_to: "#19547B", dark_from: "#5A4820", dark_to: "#0E2838" },
    BackgroundGradient { id: "flamingo", name: "Flamingo", light_from: "#EE9CA7", light_to: "#FFDDE1", dark_from: "#4A2830", dark_to: "#4E3840" },
    // Elegant & Neutral
    BackgroundGradient { id: "silver-lining", name: "Silver Lining", light_from: "#D7D2CC", light_to: "#304352", dark_from: "#3A3835", dark_to: "#141A20" },
    BackgroundGradient { id: "charcoal", name: "Charcoal", light_from: "#C9D6FF", light_to: "#E2E2E2", dark_from: "#1A2040", dark_to: "#1E1E22" },
    BackgroundGradient { id: "dusty-rose", name: "Dusty Rose", light_from: "#D4A5A5", light_to: "#F0E6E6", dark_from: "#3E2020", dark_to: "#2E2828" },
    BackgroundGradient { id: "sandstone", name: "Sandstone", light_from: "#EACDA3", light_to: "#D6AE7B", dark_from: "#3A3020", dark_to: "#3E2E1A" },
    // Vivid
    BackgroundGradient { id: "electric", name: "Electric", light_from: "#4568DC", light_to: "#B06AB3", dark_from: "#1A2250", dark_to: "#3A1A40" },
    BackgroundGradient { id: "neon-glow", name: "Neon Glow", light_from: "#FA8BFF", light_to: "#2BD2FF", dark_from: "#501858", dark_to: "#0E3848" },
    BackgroundGradient { id: "aurora", name: "Aurora", light_from: "#36D1DC", light_to: "#5B86E5", dark_from: "#0E3840", dark_to: "#1A2850" },
];

/// Look up a gradient by its persisted id.
pub fn find(id: &str) -> Option<&'static BackgroundGradient> {
    ALL.iter().find(|g| g.id == id)
}
