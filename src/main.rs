use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    execute, queue,
    style::{
        Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, ClearType},
};
use std::io::{self, Write};
use std::time::Duration;

const SIZE: usize = 8;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Cell {
    Empty,
    Black,
    White,
}

impl Cell {
    fn opponent(&self) -> Cell {
        match self {
            Cell::Black => Cell::White,
            Cell::White => Cell::Black,
            _ => Cell::Empty,
        }
    }

    fn is_empty(&self) -> bool { matches!(self, Cell::Empty) }
}

const DIRS: [(i32, i32); 8] = [
    (-1, -1), (-1, 0), (-1, 1),
    (0, -1),           (0, 1),
    (1, -1),  (1, 0),  (1, 1),
];

struct Game {
    board: [[Cell; SIZE]; SIZE],
    cursor: (usize, usize),
    // true = black's turn (human), false = white's turn (bot)
    black_turn: bool,
    valid_moves: Vec<(usize, usize)>,
    black_count: u8,
    white_count: u8,
    game_over: bool,
    status_msg: String,
    needs_clear: bool,
    last_term_size: (u16, u16),
    // cells waiting to be flipped, revealed one wave at a time
    pending_flips: Vec<Vec<(usize, usize)>>,
    flip_to_black: bool,
    animating: bool,
}

impl Game {
    fn new() -> Self {
        let mut board = [[Cell::Empty; SIZE]; SIZE];
        board[3][3] = Cell::White;
        board[3][4] = Cell::Black;
        board[4][3] = Cell::Black;
        board[4][4] = Cell::White;

        let mut g = Self {
            board,
            cursor: (3, 2),
            black_turn: true,
            valid_moves: Vec::new(),
            black_count: 2,
            white_count: 2,
            game_over: false,
            status_msg: String::from("Your turn (Black) - select a square"),
            needs_clear: true,
            last_term_size: terminal::size().unwrap_or((120, 40)),
            pending_flips: Vec::new(),
            flip_to_black: true,
            animating: false,
        };
        g.compute_valid_moves();
        g
    }

    // collects all squares where the current player can legally place
    fn compute_valid_moves(&mut self) {
        let player = if self.black_turn { Cell::Black } else { Cell::White };
        self.valid_moves = self.legal_moves_for(player);
    }

    fn legal_moves_for(&self, player: Cell) -> Vec<(usize, usize)> {
        let mut moves = Vec::new();
        for row in 0..SIZE {
            for col in 0..SIZE {
                if self.board[row][col].is_empty() && self.would_flip(row, col, player) {
                    moves.push((row, col));
                }
            }
        }
        moves
    }

    // checks if placing player at (row, col) flips at least one opponent disc
    fn would_flip(&self, row: usize, col: usize, player: Cell) -> bool {
        let opponent = player.opponent();
        for (dr, dc) in &DIRS {
            let mut r = row as i32 + dr;
            let mut c = col as i32 + dc;
            let mut found_opponent = false;
            while r >= 0 && r < SIZE as i32 && c >= 0 && c < SIZE as i32 {
                let cell = self.board[r as usize][c as usize];
                if cell == opponent {
                    found_opponent = true;
                } else if cell == player && found_opponent {
                    return true;
                } else {
                    break;
                }
                r += dr;
                c += dc;
            }
        }
        false
    }

    // collects flips grouped by wave distance from placed disc
    fn collect_flips(&self, row: usize, col: usize, player: Cell) -> Vec<Vec<(usize, usize)>> {
        let opponent = player.opponent();
        let mut by_dist: std::collections::HashMap<usize, Vec<(usize, usize)>> = std::collections::HashMap::new();

        for (dr, dc) in &DIRS {
            let mut r = row as i32 + dr;
            let mut c = col as i32 + dc;
            let mut line = Vec::new();
            while r >= 0 && r < SIZE as i32 && c >= 0 && c < SIZE as i32 {
                let cell = self.board[r as usize][c as usize];
                if cell == opponent {
                    line.push((r as usize, c as usize));
                } else if cell == player {
                    // valid line, record each cell at its distance
                    for (i, pos) in line.iter().enumerate() {
                        by_dist.entry(i).or_default().push(*pos);
                    }
                    break;
                } else {
                    break;
                }
                r += dr;
                c += dc;
            }
        }

        let mut waves: Vec<Vec<(usize, usize)>> = Vec::new();
        let max_dist = by_dist.keys().copied().max().unwrap_or(0);
        for i in 0..=max_dist {
            waves.push(by_dist.remove(&i).unwrap_or_default());
        }
        waves
    }

    // places disc, starts flip animation instead of immediately flipping
    fn place(&mut self, row: usize, col: usize) {
        let player = if self.black_turn { Cell::Black } else { Cell::White };
        self.board[row][col] = player;
        self.flip_to_black = matches!(player, Cell::Black);
        self.pending_flips = self.collect_flips(row, col, player);
        // hide valid moves immediately so they aren't visible during animation
        self.valid_moves.clear();
        self.animating = true;
    }

    // advances one wave of the flip animation; returns true when done
    fn advance_animation(&mut self) -> bool {
        if self.pending_flips.is_empty() {
            self.animating = false;
            self.finish_flip();
            return true;
        }
        let wave = self.pending_flips.remove(0);
        let target = if self.flip_to_black { Cell::Black } else { Cell::White };
        for (r, c) in wave {
            self.board[r][c] = target;
        }
        self.pending_flips.is_empty().then(|| {
            self.animating = false;
            self.finish_flip();
        });
        self.pending_flips.is_empty()
    }

    fn finish_flip(&mut self) {
        self.recount();
        self.end_turn();
    }

    fn recount(&mut self) {
        self.black_count = 0;
        self.white_count = 0;
        for row in 0..SIZE {
            for col in 0..SIZE {
                match self.board[row][col] {
                    Cell::Black => self.black_count += 1,
                    Cell::White => self.white_count += 1,
                    _ => {}
                }
            }
        }
    }

    fn end_turn(&mut self) {
        self.black_turn = !self.black_turn;
        let player = if self.black_turn { Cell::Black } else { Cell::White };
        let moves = self.legal_moves_for(player);

        if moves.is_empty() {
            let opponent_moves = self.legal_moves_for(player.opponent());
            if opponent_moves.is_empty() {
                self.game_over = true;
                let msg = if self.black_count > self.white_count {
                    "You win! Press R to restart."
                } else if self.white_count > self.black_count {
                    "Bot wins! Press R to restart."
                } else {
                    "Draw! Press R to restart."
                };
                self.status_msg = String::from(msg);
                self.valid_moves.clear();
                return;
            }
            let skipped = if self.black_turn { "Black" } else { "White" };
            self.status_msg = format!("{} has no moves - turn skipped!", skipped);
            self.black_turn = !self.black_turn;
            self.compute_valid_moves();
            return;
        }

        self.valid_moves = moves;
        if self.black_turn {
            self.status_msg = String::from("Your turn (Black) - select a square");
        } else {
            self.status_msg = String::from("Bot is thinking...");
        }
    }

    // bot uses a weighted board heuristic favoring corners and edges
    fn bot_move(&mut self) {
        let moves = self.legal_moves_for(Cell::White);
        if moves.is_empty() {
            return;
        }
        let best = moves.iter().max_by_key(|&&(r, c)| positional_weight(r, c)).cloned().unwrap();
        self.place(best.0, best.1);
    }

    fn try_place(&mut self, row: usize, col: usize) {
        if self.valid_moves.contains(&(row, col)) {
            self.place(row, col);
        } else {
            self.status_msg = String::from("Invalid move - pick a highlighted square.");
        }
    }

    fn handle_key(&mut self, key: KeyCode) {
        if self.animating { return; }

        if self.game_over {
            if key == KeyCode::Char('r') || key == KeyCode::Char('R') {
                *self = Game::new();
            }
            return;
        }

        if !self.black_turn {
            return;
        }

        let (row, col) = self.cursor;
        match key {
            KeyCode::Up => { if row > 0 { self.cursor.0 -= 1; } }
            KeyCode::Down => { if row < SIZE - 1 { self.cursor.0 += 1; } }
            KeyCode::Left => { if col > 0 { self.cursor.1 -= 1; } }
            KeyCode::Right => { if col < SIZE - 1 { self.cursor.1 += 1; } }
            KeyCode::Enter | KeyCode::Char(' ') => { self.try_place(row, col); }
            KeyCode::Char('r') | KeyCode::Char('R') => { *self = Game::new(); }
            _ => {}
        }
    }
}

// standard othello positional weight table
fn positional_weight(row: usize, col: usize) -> i32 {
    let table = [
        [120, -20, 20,  5,  5, 20, -20, 120],
        [-20, -40, -5, -5, -5, -5, -40, -20],
        [ 20,  -5, 15,  3,  3, 15,  -5,  20],
        [  5,  -5,  3,  3,  3,  3,  -5,   5],
        [  5,  -5,  3,  3,  3,  3,  -5,   5],
        [ 20,  -5, 15,  3,  3, 15,  -5,  20],
        [-20, -40, -5, -5, -5, -5, -40, -20],
        [120, -20, 20,  5,  5, 20, -20, 120],
    ];
    table[row][col]
}

const CELL_W: u16 = 5;
const CELL_H: u16 = 3;

// total board pixel width: 8 cells + 9 vertical separators (1 char each)
// each cell is CELL_W wide, separators are 1 char
// board_w = 8 * CELL_W + 9
const BOARD_PX_W: u16 = SIZE as u16 * CELL_W + 9;
// board_h = 8 * CELL_H + 9 (horizontal separators)
const BOARD_PX_H: u16 = SIZE as u16 * CELL_H + 9;

// colors
const COL_CELL: Color = Color::Rgb { r: 60, g: 100, b: 60 };
const COL_CELL_VALID: Color = Color::Rgb { r: 50, g: 130, b: 60 };
const COL_CELL_CURSOR: Color = Color::Rgb { r: 110, g: 110, b: 50 };
const COL_BORDER: Color = Color::Rgb { r: 30, g: 30, b: 30 };
const COL_TITLE: Color = Color::Rgb { r: 200, g: 200, b: 200 };
const COL_LABEL: Color = Color::Rgb { r: 100, g: 100, b: 100 };
const COL_SIDEBAR_HEAD: Color = Color::Rgb { r: 180, g: 180, b: 180 };

fn draw(stdout: &mut impl Write, game: &mut Game) -> io::Result<()> {
    let (term_w, term_h) = terminal::size().unwrap_or((120, 40));

    let resized = (term_w, term_h) != game.last_term_size;
    if resized {
        game.last_term_size = (term_w, term_h);
        game.needs_clear = true;
    }

    let total_w = 3 + BOARD_PX_W + 2 + 18;
    let total_h = 1 + 1 + BOARD_PX_H + 1 + 1 + 1 + 1;
    let ox = term_w.saturating_sub(total_w) / 2;
    let oy = term_h.saturating_sub(total_h) / 2;

    let board_x = ox + 3;
    let board_y = oy + 2;
    let bw = BOARD_PX_W;
    let bh = BOARD_PX_H;

    if game.needs_clear {
        queue!(stdout, terminal::Clear(ClearType::All))?;
        game.needs_clear = false;
        // paint border/grid lines once; cells overwrite only their interior each frame
        for dy in 0..bh {
            queue!(
                stdout,
                cursor::MoveTo(board_x, board_y + dy),
                SetBackgroundColor(COL_BORDER),
                Print(" ".repeat(bw as usize)),
                ResetColor,
            )?;
        }
    }

    // title
    queue!(
        stdout,
        cursor::MoveTo(ox + 3, oy),
        SetForegroundColor(COL_TITLE),
        SetAttribute(Attribute::Bold),
        Print("REVERSI"),
        SetAttribute(Attribute::Reset),
        ResetColor,
    )?;

    // score
    let score_x = ox + 3 + 10;
    queue!(
        stdout,
        cursor::MoveTo(score_x, oy),
        SetForegroundColor(Color::Rgb { r: 60, g: 60, b: 60 }),
        Print(format!("B:{:>2}", game.black_count)),
        SetForegroundColor(Color::Rgb { r: 90, g: 90, b: 90 }),
        Print(format!("  W:{:>2}", game.white_count)),
        ResetColor,
    )?;

    if game.needs_clear {
        queue!(stdout, terminal::Clear(ClearType::All))?;
        game.needs_clear = false;
        for dy in 0..bh {
            queue!(
                stdout,
                cursor::MoveTo(board_x, board_y + dy),
                SetBackgroundColor(COL_BORDER),
                Print(" ".repeat(bw as usize)),
                ResetColor,
            )?;
        }
    }

    // draw each cell (border persists, cells overwrite only their interior)
    for row in 0..SIZE {
        // row label
        queue!(
            stdout,
            cursor::MoveTo(ox, board_y + 1 + row as u16 * (CELL_H + 1) + CELL_H / 2),
            SetForegroundColor(COL_LABEL),
            Print(format!("{:>2}", 8 - row)),
            ResetColor,
        )?;

        for col in 0..SIZE {
            let is_cursor = game.black_turn && !game.animating && game.cursor == (row, col);
            let is_valid = !game.animating && game.valid_moves.contains(&(row, col));

            let bg = if is_cursor {
                COL_CELL_CURSOR
            } else if is_valid {
                COL_CELL_VALID
            } else {
                COL_CELL
            };

            let cx = board_x + 1 + col as u16 * (CELL_W + 1);
            let cy = board_y + 1 + row as u16 * (CELL_H + 1);

            for dy in 0..CELL_H {
                queue!(
                    stdout,
                    cursor::MoveTo(cx, cy + dy),
                    SetBackgroundColor(bg),
                    Print(" ".repeat(CELL_W as usize)),
                    ResetColor,
                )?;
            }

            let disc_x = cx + CELL_W / 2 - 1;
            let disc_y = cy + CELL_H / 2;

            match game.board[row][col] {
                Cell::Black => {
                    queue!(
                        stdout,
                        cursor::MoveTo(disc_x, disc_y),
                        SetBackgroundColor(bg),
                        SetForegroundColor(Color::Rgb { r: 10, g: 10, b: 10 }),
                        SetAttribute(Attribute::Bold),
                        Print("( )"),
                        cursor::MoveTo(disc_x + 1, disc_y),
                        SetForegroundColor(Color::Rgb { r: 30, g: 30, b: 30 }),
                        Print("●"),
                        SetAttribute(Attribute::Reset),
                        ResetColor,
                    )?;
                }
                Cell::White => {
                    queue!(
                        stdout,
                        cursor::MoveTo(disc_x, disc_y),
                        SetBackgroundColor(bg),
                        SetForegroundColor(Color::Rgb { r: 200, g: 200, b: 200 }),
                        SetAttribute(Attribute::Bold),
                        Print("( )"),
                        cursor::MoveTo(disc_x + 1, disc_y),
                        SetForegroundColor(Color::White),
                        Print("○"),
                        SetAttribute(Attribute::Reset),
                        ResetColor,
                    )?;
                }
                Cell::Empty => {
                    if is_valid {
                        queue!(
                            stdout,
                            cursor::MoveTo(cx + CELL_W / 2, disc_y),
                            SetBackgroundColor(bg),
                            SetForegroundColor(Color::Rgb { r: 80, g: 180, b: 80 }),
                            Print("·"),
                            SetAttribute(Attribute::Reset),
                            ResetColor,
                        )?;
                    }
                }
            }
        }
    }

    // column labels below board
    for col in 0..SIZE {
        let lx = board_x + 1 + col as u16 * (CELL_W + 1) + CELL_W / 2;
        queue!(
            stdout,
            cursor::MoveTo(lx, board_y + bh),
            SetForegroundColor(COL_LABEL),
            Print((b'a' + col as u8) as char),
            ResetColor,
        )?;
    }

    // status line
    let status_y = board_y + bh + 2;
    let status_color = if game.game_over {
        Color::Rgb { r: 220, g: 200, b: 80 }
    } else if !game.black_turn {
        Color::Rgb { r: 140, g: 140, b: 140 }
    } else {
        Color::Rgb { r: 180, g: 180, b: 180 }
    };
    queue!(
        stdout,
        cursor::MoveTo(ox + 3, status_y),
        terminal::Clear(ClearType::UntilNewLine),
        SetForegroundColor(status_color),
        Print(format!("{:<60}", &game.status_msg)),
        ResetColor,
    )?;

    // legend
    queue!(
        stdout,
        cursor::MoveTo(ox + 3, status_y + 1),
        SetForegroundColor(Color::Rgb { r: 70, g: 70, b: 70 }),
        Print("Arrows: move  Enter/Space: place  R: restart  Q: quit"),
        ResetColor,
    )?;

    // sidebar
    let sx = board_x + bw + 2;
    let sy = board_y;

    queue!(
        stdout,
        cursor::MoveTo(sx, sy + 1),
        SetForegroundColor(COL_SIDEBAR_HEAD),
        SetAttribute(Attribute::Bold),
        Print("TURN"),
        SetAttribute(Attribute::Reset),
        ResetColor,
    )?;

    let you_active = game.black_turn && !game.game_over;
    let bot_active = !game.black_turn && !game.game_over;

    queue!(
        stdout,
        cursor::MoveTo(sx, sy + 3),
        SetForegroundColor(if you_active { Color::Rgb { r: 220, g: 220, b: 220 } } else { Color::Rgb { r: 70, g: 70, b: 70 } }),
        SetAttribute(if you_active { Attribute::Bold } else { Attribute::Reset }),
        Print(if you_active { "▶ You (Black)" } else { "  You (Black)" }),
        SetAttribute(Attribute::Reset),
        ResetColor,
    )?;

    queue!(
        stdout,
        cursor::MoveTo(sx, sy + 4),
        SetForegroundColor(if bot_active { Color::Rgb { r: 220, g: 220, b: 220 } } else { Color::Rgb { r: 70, g: 70, b: 70 } }),
        SetAttribute(if bot_active { Attribute::Bold } else { Attribute::Reset }),
        Print(if bot_active { "▶ Bot (White)" } else { "  Bot (White)" }),
        SetAttribute(Attribute::Reset),
        ResetColor,
    )?;

    queue!(
        stdout,
        cursor::MoveTo(sx, sy + 7),
        SetForegroundColor(Color::Rgb { r: 80, g: 80, b: 80 }),
        Print("Discs"),
        ResetColor,
    )?;
    queue!(
        stdout,
        cursor::MoveTo(sx, sy + 8),
        SetForegroundColor(Color::Rgb { r: 50, g: 50, b: 50 }),
        SetBackgroundColor(Color::Rgb { r: 160, g: 160, b: 160 }),
        Print(" ● "),
        ResetColor,
        SetForegroundColor(Color::Rgb { r: 80, g: 80, b: 80 }),
        Print(" black"),
        ResetColor,
    )?;
    queue!(
        stdout,
        cursor::MoveTo(sx, sy + 9),
        SetForegroundColor(Color::Rgb { r: 220, g: 220, b: 220 }),
        SetBackgroundColor(Color::Rgb { r: 160, g: 160, b: 160 }),
        Print(" ○ "),
        ResetColor,
        SetForegroundColor(Color::Rgb { r: 80, g: 80, b: 80 }),
        Print(" white"),
        ResetColor,
    )?;

    stdout.flush()?;
    Ok(())
}

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide)?;

    let mut game = Game::new();
    // ms between flip animation waves
    let flip_interval = Duration::from_millis(120);
    let mut last_flip = std::time::Instant::now();

    loop {
        draw(&mut stdout, &mut game)?;

        // advance flip animation on timer
        if game.animating {
            if last_flip.elapsed() >= flip_interval {
                last_flip = std::time::Instant::now();
                let done = game.advance_animation();
                // when animation finishes and it's bot's turn, fire immediately
                if done && !game.black_turn && !game.game_over {
                    game.bot_move();
                    last_flip = std::time::Instant::now();
                }
            }
            // drain events during animation so input doesn't queue up
            while event::poll(Duration::from_millis(0))? {
                if let Event::Resize(_, _) = event::read()? {
                    game.needs_clear = true;
                }
            }
            std::thread::sleep(Duration::from_millis(16));
            continue;
        }

        // bot's turn: place immediately, animation takes over
        if !game.black_turn && !game.game_over {
            game.bot_move();
            last_flip = std::time::Instant::now();
            continue;
        }

        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(KeyEvent { code, kind: KeyEventKind::Press, .. }) => {
                    match code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => break,
                        other => {
                            game.handle_key(other);
                            if game.animating {
                                last_flip = std::time::Instant::now();
                            }
                        }
                    }
                }
                Event::Resize(_, _) => {
                    game.needs_clear = true;
                }
                _ => {}
            }
        }
    }

    execute!(stdout, terminal::LeaveAlternateScreen, cursor::Show)?;
    terminal::disable_raw_mode()?;
    Ok(())
}
