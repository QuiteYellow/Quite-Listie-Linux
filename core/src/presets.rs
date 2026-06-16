pub struct LabelPreset {
    pub id: &'static str,
    pub name: &'static str,
    pub color: &'static str,
}

pub const GROCERY_LABELS: &[LabelPreset] = &[
    LabelPreset { id: "produce",       name: "Produce",       color: "#4CAF50" },
    LabelPreset { id: "dairy",         name: "Dairy",         color: "#2196F3" },
    LabelPreset { id: "meat",          name: "Meat",          color: "#F44336" },
    LabelPreset { id: "bakery",        name: "Bakery",        color: "#FF9800" },
    LabelPreset { id: "frozen",        name: "Frozen",        color: "#00BCD4" },
    LabelPreset { id: "pantry",        name: "Pantry",        color: "#9C27B0" },
    LabelPreset { id: "snacks",        name: "Snacks",        color: "#FFEB3B" },
    LabelPreset { id: "beverages",     name: "Beverages",     color: "#795548" },
    LabelPreset { id: "household",     name: "Household",     color: "#607D8B" },
    LabelPreset { id: "personal-care", name: "Personal Care", color: "#E91E63" },
];
