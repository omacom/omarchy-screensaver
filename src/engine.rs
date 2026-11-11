use std::{fs, path::Path};

use anyhow::{Context, Result, anyhow, bail};
use clap::{CommandFactory, Parser};
use ttfx::{
    cli::Cli,
    engine::{
        canvas::Anchor,
        ctx::{Clock, EngineCtx},
        effect::Effect,
        terminal::TerminalConfig,
    },
    utils::rng::Rng,
};

use crate::{
    options::Options,
    vt::{self, Cell},
};

struct RunningEffect {
    ctx: EngineCtx,
    effect: Box<dyn Effect>,
    name: String,
}

pub struct Animation {
    input: String,
    options: Options,
    running: RunningEffect,
    cells: Vec<Cell>,
    pending_frame: Option<String>,
    cols: i32,
    rows: i32,
}

impl Animation {
    pub fn new(input: String, options: Options, cols: i32, rows: i32) -> Result<Self> {
        let cols = cols.max(1);
        let rows = rows.max(1);
        let running = build_effect(&input, &options, cols, rows)?;
        Ok(Self {
            input,
            options,
            running,
            cells: vec![Cell::default(); cols as usize * rows as usize],
            pending_frame: None,
            cols,
            rows,
        })
    }

    pub fn resize(&mut self, cols: i32, rows: i32) -> Result<()> {
        let (cols, rows) = (cols.max(1), rows.max(1));
        if (cols, rows) == (self.cols, self.rows) {
            return Ok(());
        }
        self.cols = cols;
        self.rows = rows;
        self.cells
            .resize(cols as usize * rows as usize, Cell::default());
        self.pending_frame = None;
        self.running = build_effect(&self.input, &self.options, cols, rows)?;
        Ok(())
    }

    /// Advances and rasterizes exactly one ttfx simulation frame.
    /// Returns whether the resulting cell grid changed.
    pub fn advance(&mut self) -> Result<bool> {
        self.advance_deferred()?;
        self.rasterize_pending()
    }

    /// Advances the simulation while deferring VT parsing until presentation.
    pub fn advance_deferred(&mut self) -> Result<()> {
        self.pending_frame = Some(self.next_frame()?);
        Ok(())
    }

    /// Rasterizes the latest deferred frame, if any.
    pub fn rasterize_pending(&mut self) -> Result<bool> {
        let Some(frame) = self.pending_frame.take() else {
            return Ok(false);
        };
        vt::raster(frame.as_bytes(), &mut self.cells, self.cols, self.rows)
            .map_err(anyhow::Error::msg)
    }

    fn next_frame(&mut self) -> Result<String> {
        match self.running.effect.next_frame(&mut self.running.ctx) {
            Some(frame) => Ok(frame),
            None => {
                self.running = build_effect(&self.input, &self.options, self.cols, self.rows)?;
                self.running
                    .effect
                    .next_frame(&mut self.running.ctx)
                    .ok_or_else(|| anyhow!("ttfx effect '{}' produced no frame", self.running.name))
            }
        }
    }

    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    pub fn cols(&self) -> i32 {
        self.cols
    }

    pub fn rows(&self) -> i32 {
        self.rows
    }

    pub fn effect_name(&self) -> &str {
        &self.running.name
    }
}

fn build_effect(input: &str, options: &Options, cols: i32, rows: i32) -> Result<RunningEffect> {
    let mut rng = options
        .seed
        .map(Rng::seeded)
        .unwrap_or_else(Rng::from_entropy);
    let (name, command) = pick_effect(
        options.effect.as_deref(),
        &options.include_effects,
        &options.exclude_effects,
        &mut rng,
    )?;
    let config = terminal_config(options, cols, rows);
    let clock = Clock::virtual_with_frame_rate(config.frame_rate);
    let mut ctx = EngineCtx::new(input, config, rng, clock)
        .map_err(|error| anyhow!("EngineCtx::new: {error}"))?;
    let mut effect = command.build_effect();
    effect
        .build(&mut ctx)
        .map_err(|error| anyhow!("effect.build: {error}"))?;
    Ok(RunningEffect { ctx, effect, name })
}

fn effect_names() -> Vec<String> {
    Cli::command()
        .get_subcommands()
        .map(|command| command.get_name().to_owned())
        .collect()
}

fn effect_by_name(name: &str) -> Result<ttfx::effects::EffectCommand> {
    match Cli::try_parse_from(["ttfx", name]) {
        Ok(Cli {
            effect: Some(effect),
            ..
        }) => Ok(effect),
        Ok(_) => bail!("no effect in parse of '{name}'"),
        Err(error) => bail!("unknown effect '{name}': {error}"),
    }
}

fn pick_effect(
    requested: Option<&str>,
    include: &[String],
    exclude: &[String],
    rng: &mut Rng,
) -> Result<(String, ttfx::effects::EffectCommand)> {
    let all = effect_names();
    if all.is_empty() {
        bail!("ttfx has no effects");
    }
    let name = match requested {
        None | Some("") | Some("random") => {
            for item in include {
                if !all.contains(item) {
                    bail!("unknown effect in --include-effects: '{item}'");
                }
            }
            for item in exclude {
                if !all.contains(item) {
                    bail!("unknown effect in --exclude-effects: '{item}'");
                }
            }
            let mut names = all;
            if !include.is_empty() {
                names.retain(|name| include.contains(name));
            }
            names.retain(|name| !exclude.contains(name));
            if names.is_empty() {
                bail!("no effects left after include/exclude");
            }
            names[rng.choice_index(names.len())].clone()
        }
        Some(name) => {
            if !all.iter().any(|available| available == name) {
                bail!("unknown effect '{name}' (have: {})", all.join(", "));
            }
            name.to_owned()
        }
    };
    let command = effect_by_name(&name).with_context(|| format!("selecting effect '{name}'"))?;
    Ok((name, command))
}

/// ttfx steps and the overlay redraws at this rate.
pub const FRAME_RATE: u32 = 120;

/// On battery the overlay redraws at this rate instead. ttfx keeps stepping at
/// `FRAME_RATE`, so every effect runs at its usual speed.
pub const BATTERY_PRESENT_RATE: u32 = 30;

/// The rate the overlay redraws at: slower while a battery is discharging,
/// because each redraw makes the compositor present the whole screen again.
pub fn present_rate() -> u32 {
    if discharging(Path::new("/sys/class/power_supply")) {
        BATTERY_PRESENT_RATE
    } else {
        FRAME_RATE
    }
}

fn discharging(power_supplies: &Path) -> bool {
    let Ok(entries) = fs::read_dir(power_supplies) else {
        return false;
    };
    // A wireless mouse or keyboard reports its own battery here, with a scope
    // of Device. Only a battery that powers the machine counts.
    entries.flatten().any(|entry| {
        let read = |name: &str| fs::read_to_string(entry.path().join(name)).unwrap_or_default();
        read("status").trim() == "Discharging" && read("scope").trim() != "Device"
    })
}

fn terminal_config(options: &Options, cols: i32, rows: i32) -> TerminalConfig {
    TerminalConfig {
        frame_rate: i64::from(FRAME_RATE),
        canvas_width: i64::from(if options.canvas_width > 0 {
            options.canvas_width
        } else {
            cols
        }),
        canvas_height: i64::from(if options.canvas_height > 0 {
            options.canvas_height
        } else {
            rows
        }),
        ignore_terminal_dimensions: true,
        reuse_canvas: options.reuse_canvas,
        no_eol: options.no_eol,
        no_restore_cursor: options.no_restore_cursor,
        anchor_canvas: Anchor::parse(&options.anchor_canvas).unwrap_or(Anchor::C),
        anchor_text: Anchor::parse(&options.anchor_text).unwrap_or(Anchor::C),
        ..TerminalConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supplies(name: &str, statuses: &[(&str, &str)]) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!("screensaver-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for (supply, status) in statuses {
            fs::create_dir_all(root.join(supply)).unwrap();
            fs::write(root.join(supply).join("status"), format!("{status}\n")).unwrap();
        }
        fs::create_dir_all(root.join("AC")).unwrap();
        root
    }

    #[test]
    fn a_discharging_battery_slows_the_redraws() {
        let root = supplies("discharging", &[("BAT0", "Discharging")]);
        assert!(discharging(&root));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_charging_or_absent_battery_keeps_the_full_rate() {
        let root = supplies("charging", &[("BAT0", "Charging"), ("BAT1", "Full")]);
        assert!(!discharging(&root));
        fs::remove_dir_all(root).unwrap();
        assert!(!discharging(Path::new("/nonexistent/power_supply")));
    }

    #[test]
    fn a_discharging_peripheral_keeps_the_full_rate() {
        let root = supplies(
            "peripheral",
            &[("BAT0", "Charging"), ("hidpp_battery_0", "Discharging")],
        );
        fs::write(root.join("hidpp_battery_0").join("scope"), "Device\n").unwrap();
        assert!(!discharging(&root));
        fs::remove_dir_all(root).unwrap();
    }
}
