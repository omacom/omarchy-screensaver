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
    observer::{MAX_INPUT_BYTES, Snapshot},
    options::Options,
    vt::{self, Cell},
};

struct RunningEffect {
    ctx: EngineCtx,
    effect: Box<dyn Effect>,
    name: String,
    rng_state: [u64; 4],
}

pub struct Animation {
    input: String,
    options: Options,
    running: RunningEffect,
    cells: Vec<Cell>,
    pending_frame: Option<String>,
    cols: i32,
    rows: i32,
    effect_started: bool,
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
            effect_started: true,
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
        self.effect_started = true;
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
                self.effect_started = true;
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

    /// Take the current initialization once after construction or a rebuild.
    pub fn take_started_effect(&mut self) -> Option<Snapshot> {
        if !std::mem::take(&mut self.effect_started) {
            return None;
        }
        if self.input.len() > MAX_INPUT_BYTES {
            return Some(Snapshot::default());
        }
        Some(Snapshot {
            active: true,
            engine: "ttfx-0.5.1".into(),
            effect: self.running.name.clone(),
            input: self.input.clone(),
            rng_state: self.running.rng_state.to_vec(),
            ..Snapshot::default()
        })
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
    let rng_state = rng.state();
    let config = terminal_config(options, cols, rows);
    let clock = Clock::virtual_with_frame_rate(config.frame_rate);
    let mut ctx = EngineCtx::new(input, config, rng, clock)
        .map_err(|error| anyhow!("EngineCtx::new: {error}"))?;
    let mut effect = command.build_effect();
    effect
        .build(&mut ctx)
        .map_err(|error| anyhow!("effect.build: {error}"))?;
    Ok(RunningEffect {
        ctx,
        effect,
        name,
        rng_state,
    })
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
mod observer_tests {
    use super::*;

    #[test]
    fn published_input_is_bounded_in_bytes_without_changing_animation_input() {
        let boundary = "é".repeat(MAX_INPUT_BYTES / 2);
        for input in [
            format!("{}x", "é".repeat(MAX_INPUT_BYTES / 2 - 1)),
            boundary.clone(),
            format!("{boundary}x"),
        ] {
            let options = Options {
                effect: Some("print".into()),
                seed: Some(42),
                ..Options::default()
            };
            let mut animation = Animation::new(input.clone(), options, 40, 10).unwrap();
            let snapshot = animation.take_started_effect().unwrap();
            if input.len() <= MAX_INPUT_BYTES {
                assert!(snapshot.active);
                assert_eq!(snapshot.input, input);
            } else {
                assert_eq!(snapshot, Snapshot::default());
            }
            assert_eq!(animation.input, input);
            assert!(animation.take_started_effect().is_none());
        }
    }

    #[test]
    fn completion_makes_one_new_description_available() {
        let options = Options {
            effect: Some("print".into()),
            seed: Some(42),
            ..Options::default()
        };
        let mut animation = Animation::new("A".into(), options, 4, 2).unwrap();
        animation.take_started_effect().unwrap();
        for _ in 0..1000 {
            animation.advance_deferred().unwrap();
            if let Some(effect) = animation.take_started_effect() {
                assert_eq!(effect.effect, "print");
                assert!(animation.take_started_effect().is_none());
                return;
            }
        }
        panic!("effect never completed");
    }

    #[test]
    fn published_rng_reconstructs_the_selected_effect_before_layout_initialization() {
        for name in ["rain", "matrix", "print", "random"] {
            let options = Options {
                effect: (name != "random").then(|| name.into()),
                include_effects: vec!["rain".into(), "matrix".into(), "print".into()],
                seed: Some(42),
                ..Options::default()
            };
            let mut animation = Animation::new("OMARCHY".into(), options.clone(), 40, 10).unwrap();
            let snapshot = animation.take_started_effect().unwrap();
            assert!(animation.take_started_effect().is_none());
            if name != "random" {
                assert_eq!(snapshot.effect, name);
            }
            let config = terminal_config(&options, 40, 10);
            let clock = Clock::virtual_with_frame_rate(config.frame_rate);
            let mut ctx = EngineCtx::new(
                &snapshot.input,
                config,
                Rng::from_state(snapshot.rng_state.try_into().unwrap()),
                clock,
            )
            .unwrap();
            let mut effect = effect_by_name(&snapshot.effect).unwrap().build_effect();
            effect.build(&mut ctx).unwrap();
            for _ in 0..30 {
                let Some(expected) = effect.next_frame(&mut ctx) else {
                    break;
                };
                assert_eq!(animation.next_frame().unwrap(), expected);
                assert!(animation.take_started_effect().is_none());
            }
            animation.resize(60, 12).unwrap();
            assert_eq!(
                animation.take_started_effect().unwrap().effect,
                snapshot.effect
            );
            animation.resize(60, 12).unwrap();
            assert!(animation.take_started_effect().is_none());
        }
    }
}
