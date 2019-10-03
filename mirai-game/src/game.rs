use crate::inputs::{Input, InputSource};
use ggez::graphics::Color;
use ggez::nalgebra as na;
use ggez::*;
use PlayerState::*;

const FRAMES_PER_SECOND: u32 = 32;
const PLAYER_SIZE: i32 = 32;
const MOVESPEED: i32 = 4;
const GROUND_LEVEL: i32 = 360;
const GROUND_SIZE: i32 = 480;
const GROUND_X_START: i32 = 80;
const GROUND_X_END: i32 = GROUND_X_START + GROUND_SIZE;

#[derive(Clone)]
struct State {
    frame: u32,
    p1: Player,
    p2: Player,
}

pub struct Game {
    current_frame: u32,
    target_frame: u32,
    p1: Player,
    p2: Player,
    input_source: InputSource,
    last_confirmed_state: State,
}

impl Game {
    pub fn new(input_source: InputSource) -> Self {
        let p1 = Player::new(160 - PLAYER_SIZE / 2, GROUND_LEVEL, Side::Left);
        let p2 = Player::new(480 - PLAYER_SIZE / 2, GROUND_LEVEL, Side::Right);
        let confirmed_state = State {
            frame: 0,
            p1: p1.clone(),
            p2: p2.clone(),
        };
        let mut s = Game {
            current_frame: 0,
            target_frame: 0,
            p1: p1,
            p2: p2,
            input_source,
            last_confirmed_state: confirmed_state.clone(),
        };
        s
    }

    fn reset(&mut self) {
        self.p1 = Player::new(160 - PLAYER_SIZE / 2, GROUND_LEVEL, Side::Left);
        self.p2 = Player::new(480 - PLAYER_SIZE / 2, GROUND_LEVEL, Side::Right);
    }

    // saves the current state
    fn save_state(&mut self) {
        self.last_confirmed_state = State {
            frame: self.current_frame,
            p1: self.p1.clone(),
            p2: self.p2.clone(),
        };
    }

    // loads the previous confirmed state
    fn load_state(&mut self) {
        let confirmed = &self.last_confirmed_state;
        self.current_frame = confirmed.frame;
        self.p1 = confirmed.p1.clone();
        self.p2 = confirmed.p2.clone();
    }
}

#[derive(Clone, Copy)]
enum Side {
    Left,
    Right,
}

impl event::EventHandler for Game {
    fn update(&mut self, ctx: &mut Context) -> GameResult<()> {
        // fixed timer, controls the game tick rate
        while timer::check_update_time(ctx, FRAMES_PER_SECOND) {
            self.target_frame += 1;
            println!("progressed to {}", self.target_frame);
            // lets the input source know we are progressing the frame now
            self.input_source.progress_frame(ctx, self.target_frame);
        }

        // check rollback
        let latest_fully_confirmed = self.input_source.latest_fully_confirmed();
        if latest_fully_confirmed + 8 < self.target_frame {
            panic!("no inputs received for the past 8 frames, giving up");
        }
        if latest_fully_confirmed > self.last_confirmed_state.frame
            && self.current_frame > self.last_confirmed_state.frame
        {
            // the input source has confirmed frames further than the game: rollback to sync
            self.load_state();
        }

        // progress game state from current to target frame
        while self.current_frame < self.target_frame {
            self.current_frame += 1;
            // get inputs for current frame
            let p1_inputs = self.input_source.p1_input_for(self.current_frame);
            let p2_inputs = self.input_source.p2_input_for(self.current_frame);

            // orient players, no change on p1.x == p2.x because both players think they are p1 so it's impossible to do consistently
            if self.p1.x < self.p2.x {
                self.p1.side = Side::Left;
                self.p2.side = Side::Right;
            } else if self.p1.x > self.p2.x {
                self.p2.side = Side::Left;
                self.p1.side = Side::Right;
            }
            handle_player(&mut self.p1, &p1_inputs, &mut self.p2);
            handle_player(&mut self.p2, &p2_inputs, &mut self.p1);
            check_hitboxes(&mut self.p1, &mut self.p2);
            check_hitboxes(&mut self.p2, &mut self.p1);

            if self.p1.dead() {
                println!("player 1 died");
                self.reset();
            }
            if self.p2.dead() {
                println!("player 2 died");
                self.reset();
            }

            if self.current_frame == latest_fully_confirmed {
                // reached latest fully confirmed frame, saving state
                self.save_state();
            }
        }
        Ok(())
    }

    fn draw(&mut self, ctx: &mut Context) -> GameResult<()> {
        graphics::clear(ctx, graphics::WHITE);

        // draw ground
        let ground = graphics::Mesh::new_rectangle(
            ctx,
            graphics::DrawMode::fill(),
            graphics::Rect::new(
                80.0,
                (GROUND_LEVEL + PLAYER_SIZE) as f32,
                GROUND_SIZE as f32,
                16.0,
            ),
            graphics::BLACK,
        )?;
        graphics::draw(ctx, &ground, (na::Point2::new(0.0, 0.0),))?;

        // draw p1
        let player_square = graphics::Mesh::new_rectangle(
            ctx,
            graphics::DrawMode::fill(),
            graphics::Rect::new(0.0, 0.0, 32.0, 32.0),
            Color::new(0.0, 1.0, 1.0, 1.0),
        )?;
        graphics::draw(
            ctx,
            &player_square,
            (na::Point2::new(self.p1.x as f32, self.p1.y as f32),),
        )?;
        // draw p2
        let player_square = graphics::Mesh::new_rectangle(
            ctx,
            graphics::DrawMode::fill(),
            graphics::Rect::new(0.0, 0.0, 32.0, 32.0),
            Color::new(1.0, 0.0, 1.0, 1.0),
        )?;
        graphics::draw(
            ctx,
            &player_square,
            (na::Point2::new(self.p2.x as f32, self.p2.y as f32),),
        )?;

        // draw hurtboxes
        for hurtbox in self.p1.hurtboxes.iter().chain(self.p2.hurtboxes.iter()) {
            let square = graphics::Mesh::new_rectangle(
                ctx,
                graphics::DrawMode::fill(),
                graphics::Rect::new(0.0, 0.0, hurtbox.width as f32, hurtbox.height as f32),
                Color::new(0.0, 0.0, 1.0, 0.4),
            )?;
            graphics::draw(
                ctx,
                &square,
                (na::Point2::new(hurtbox.x as f32, hurtbox.y as f32),),
            )?;
        }
        // draw hitboxes
        for hitbox in self.p1.hitboxes.iter().chain(self.p2.hitboxes.iter()) {
            let square = graphics::Mesh::new_rectangle(
                ctx,
                graphics::DrawMode::fill(),
                graphics::Rect::new(0.0, 0.0, hitbox.width as f32, hitbox.height as f32),
                Color::new(1.0, 0.0, 0.0, 0.4),
            )?;
            graphics::draw(
                ctx,
                &square,
                (na::Point2::new(hitbox.x as f32, hitbox.y as f32),),
            )?;
        }

        graphics::present(ctx)
    }
}

fn check_hitboxes(p: &Player, opp: &mut Player) {
    let side_mult = match p.side {
        Side::Left => 1,
        Side::Right => -1,
    };
    for hitbox in &p.hitboxes {
        for hurtbox in &opp.hurtboxes {
            if hitbox.touches(hurtbox) {
                opp.health -= 1;
                opp.state = Midair(Momentum {
                    x: 8 * side_mult,
                    y: -8,
                });
            }
        }
    }
}

fn handle_player(p: &mut Player, inp: &Input, opp: &Player) {
    // clear hit and hurtboxes
    p.hitboxes.clear();
    p.hurtboxes.clear();

    if let Midair(mom) = &mut p.state {
        p.y += mom.y;
        p.x += mom.x;
        mom.y += 2;
    }
    if let Midair(_) = &p.state {
        if p.inside_ground() && p.y >= GROUND_LEVEL {
            p.y = GROUND_LEVEL;
            p.state = Standing;
        }
    }
    if let Standing = &mut p.state {
        if inp.attack {
            p.state = Attacking(0);
        } else {
            if inp.left {
                p.x -= MOVESPEED;
                if !p.inside_ground() {
                    p.state = Midair(Momentum {
                        x: -MOVESPEED,
                        y: 2,
                    });
                }
            }
            if inp.right {
                p.x += MOVESPEED;
                if !p.inside_ground() {
                    p.state = Midair(Momentum { x: MOVESPEED, y: 2 });
                }
            }
        }
    }
    if let Attacking(att_frame) = &mut p.state {
        if *att_frame == 5 {
            // active
            let side_mult = match p.side {
                Side::Left => 1,
                Side::Right => -1,
            };
            // set the hitbox
            p.hitboxes.push(Hitbox::new(
                p.x + PLAYER_SIZE * side_mult,
                p.y + PLAYER_SIZE / 4,
                PLAYER_SIZE,
                PLAYER_SIZE / 2,
            ));
            *att_frame += 1;
        } else if *att_frame < 16 {
            // startup or recovery
            *att_frame += 1;
        } else {
            // finished
            p.state = Standing;
        }
    }
    // set the hurtbox
    p.hurtboxes.push(Hitbox::for_player(&p));
}

#[derive(Clone)]
struct Player {
    x: i32,
    y: i32,
    side: Side,
    health: i32,
    hurtboxes: Vec<Hitbox>,
    hitboxes: Vec<Hitbox>,
    state: PlayerState,
}

impl Player {
    pub fn new(x: i32, y: i32, side: Side) -> Self {
        Player {
            x,
            y,
            side,
            health: 4,
            hurtboxes: Vec::new(),
            hitboxes: Vec::new(),
            state: PlayerState::Standing,
        }
    }

    pub fn inside_ground(&self) -> bool {
        self.x + PLAYER_SIZE > GROUND_X_START && self.x < GROUND_X_END
    }

    pub fn dead(&self) -> bool {
        self.y > GROUND_LEVEL + 100 || self.health <= 0
    }
}

#[derive(Clone, Copy)]
struct Hitbox {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl Hitbox {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Hitbox {
            x,
            y,
            width,
            height,
        }
    }

    pub fn for_player(owner: &Player) -> Self {
        Self::new(owner.x, owner.y, PLAYER_SIZE, PLAYER_SIZE)
    }

    pub fn touches(&self, hitbox: &Hitbox) -> bool {
        self.x < hitbox.x + hitbox.width
            && self.x + self.width > hitbox.x
            && self.y < hitbox.y + hitbox.height
            && self.y + self.height > hitbox.y
    }
}

#[derive(Clone, Copy)]
enum PlayerState {
    Standing,
    Midair(Momentum),
    Attacking(u32),
}

#[derive(Clone, Copy)]
struct Momentum {
    x: i32,
    y: i32,
}
