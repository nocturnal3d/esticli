use colorgrad::{preset, Gradient};
use ratatui::style::Color;
use std::fmt;
use std::str::FromStr;

// Available colormaps for gradient visualization
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Colormap {
    #[default]
    Turbo,
    Spectral,
    Inferno,
    Magma,
    Plasma,
    Viridis,
    Rainbow,
    Cividis,
    Warm,
    Cool,
}

impl Colormap {
    pub const ALL: &'static [Colormap] = &[
        Colormap::Inferno,
        Colormap::Magma,
        Colormap::Plasma,
        Colormap::Viridis,
        Colormap::Turbo,
        Colormap::Spectral,
        Colormap::Rainbow,
        Colormap::Cividis,
        Colormap::Warm,
        Colormap::Cool,
    ];

    pub fn next(&self) -> Self {
        match self {
            Colormap::Inferno => Colormap::Magma,
            Colormap::Magma => Colormap::Plasma,
            Colormap::Plasma => Colormap::Viridis,
            Colormap::Viridis => Colormap::Turbo,
            Colormap::Turbo => Colormap::Spectral,
            Colormap::Spectral => Colormap::Rainbow,
            Colormap::Rainbow => Colormap::Cividis,
            Colormap::Cividis => Colormap::Warm,
            Colormap::Warm => Colormap::Cool,
            Colormap::Cool => Colormap::Inferno,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            Colormap::Inferno => Colormap::Cool,
            Colormap::Magma => Colormap::Inferno,
            Colormap::Plasma => Colormap::Magma,
            Colormap::Viridis => Colormap::Plasma,
            Colormap::Turbo => Colormap::Viridis,
            Colormap::Spectral => Colormap::Turbo,
            Colormap::Rainbow => Colormap::Spectral,
            Colormap::Cividis => Colormap::Rainbow,
            Colormap::Warm => Colormap::Cividis,
            Colormap::Cool => Colormap::Warm,
        }
    }

    // Generate a color from this colormap at a given position (0.0 to 1.0)
    pub fn color_at(&self, position: f32) -> Color {
        let t = position.clamp(0.0, 1.0);
        let rgba = match self {
            Colormap::Turbo => preset::turbo().at(1.0 - t).to_rgba8(),
            Colormap::Spectral => preset::spectral().at(1.0 - t).to_rgba8(),
            Colormap::Inferno => preset::inferno().at(1.0 - t).to_rgba8(),
            Colormap::Magma => preset::magma().at(1.0 - t).to_rgba8(),
            Colormap::Plasma => preset::plasma().at(1.0 - t).to_rgba8(),
            Colormap::Viridis => preset::viridis().at(1.0 - t).to_rgba8(),
            Colormap::Rainbow => preset::rainbow().at(1.0 - t).to_rgba8(),
            Colormap::Cividis => preset::cividis().at(1.0 - t).to_rgba8(),
            Colormap::Warm => preset::warm().at(1.0 - t).to_rgba8(),
            Colormap::Cool => preset::cool().at(1.0 - t).to_rgba8(),
        };
        let [r, g, b, _] = rgba;
        Color::Rgb(r, g, b)
    }
}

impl fmt::Display for Colormap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Colormap::Turbo => write!(f, "turbo"),
            Colormap::Spectral => write!(f, "spectral"),
            Colormap::Inferno => write!(f, "inferno"),
            Colormap::Magma => write!(f, "magma"),
            Colormap::Plasma => write!(f, "plasma"),
            Colormap::Viridis => write!(f, "viridis"),
            Colormap::Rainbow => write!(f, "rainbow"),
            Colormap::Cividis => write!(f, "cividis"),
            Colormap::Warm => write!(f, "warm"),
            Colormap::Cool => write!(f, "cool"),
        }
    }
}

impl FromStr for Colormap {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "turbo" => Ok(Colormap::Turbo),
            "spectral" => Ok(Colormap::Spectral),
            "inferno" => Ok(Colormap::Inferno),
            "magma" => Ok(Colormap::Magma),
            "plasma" => Ok(Colormap::Plasma),
            "viridis" => Ok(Colormap::Viridis),
            "rainbow" => Ok(Colormap::Rainbow),
            "cividis" => Ok(Colormap::Cividis),
            "warm" => Ok(Colormap::Warm),
            "cool" => Ok(Colormap::Cool),
            _ => Err(format!(
                "Unknown colormap '{}'. Available: {}",
                s,
                Colormap::ALL
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortColumn {
    Name,
    DocCount,
    #[default]
    Rate,
    Size,
    Health,
}

impl SortColumn {
    pub fn next(&self) -> Self {
        match self {
            SortColumn::Name => SortColumn::DocCount,
            SortColumn::DocCount => SortColumn::Rate,
            SortColumn::Rate => SortColumn::Size,
            SortColumn::Size => SortColumn::Health,
            SortColumn::Health => SortColumn::Name,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            SortColumn::Name => SortColumn::Health,
            SortColumn::DocCount => SortColumn::Name,
            SortColumn::Rate => SortColumn::DocCount,
            SortColumn::Size => SortColumn::Rate,
            SortColumn::Health => SortColumn::Size,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortOrder {
    Ascending,
    #[default]
    Descending,
}

impl SortOrder {
    pub fn toggle(&self) -> Self {
        match self {
            SortOrder::Ascending => SortOrder::Descending,
            SortOrder::Descending => SortOrder::Ascending,
        }
    }
}
