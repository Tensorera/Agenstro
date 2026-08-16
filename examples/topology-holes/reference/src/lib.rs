//! A small, deterministic digital-topology reference implementation.
//!
//! Foreground (`#`) components use four-neighbour connectivity. Background
//! (`.`) components use the dual eight-neighbour connectivity, so a diagonal
//! opening connects a cavity to the exterior instead of creating an ambiguous
//! digital hole.

use std::collections::VecDeque;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Grid {
    width: usize,
    height: usize,
    cells: Vec<Cell>,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Cell {
    Solid,
    Background,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseGridError {
    message: String,
}

impl Display for ParseGridError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for ParseGridError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Topology {
    pub width: usize,
    pub height: usize,
    pub solid_components: usize,
    pub holes: usize,
    pub euler_characteristic: i64,
    pub solid_cells: usize,
    pub background_cells: usize,
    pub hole_areas: Vec<usize>,
}

impl Grid {
    pub fn parse(input: &str) -> Result<Self, ParseGridError> {
        let lines: Vec<&str> = input.lines().map(|line| line.trim_end_matches('\r')).collect();
        if lines.is_empty() {
            return Err(error("grid must contain at least one row"));
        }

        let width = lines[0].chars().count();
        if width == 0 {
            return Err(error("grid rows must not be empty"));
        }

        let height = lines.len();
        let capacity = width
            .checked_mul(height)
            .ok_or_else(|| error("grid dimensions overflow addressable memory"))?;
        let mut cells = Vec::with_capacity(capacity);

        for (row, line) in lines.iter().enumerate() {
            let actual_width = line.chars().count();
            if actual_width != width {
                return Err(error(format!(
                    "row {} has width {}, expected {}",
                    row + 1,
                    actual_width,
                    width
                )));
            }
            for (column, value) in line.chars().enumerate() {
                cells.push(match value {
                    '#' => Cell::Solid,
                    '.' => Cell::Background,
                    other => {
                        return Err(error(format!(
                            "row {}, column {} contains unsupported character {other:?}",
                            row + 1,
                            column + 1
                        )));
                    }
                });
            }
        }

        Ok(Self {
            width,
            height,
            cells,
        })
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    fn index(&self, row: usize, column: usize) -> usize {
        row * self.width + column
    }
}

pub fn analyze(grid: &Grid) -> Topology {
    let solid_components = count_solid_components(grid);
    let mut hole_areas = background_hole_areas(grid);
    hole_areas.sort_unstable();
    let holes = hole_areas.len();
    let solid_cells = grid
        .cells
        .iter()
        .filter(|cell| **cell == Cell::Solid)
        .count();
    let background_cells = grid.cells.len() - solid_cells;

    Topology {
        width: grid.width,
        height: grid.height,
        solid_components,
        holes,
        euler_characteristic: solid_components as i64 - holes as i64,
        solid_cells,
        background_cells,
        hole_areas,
    }
}

fn count_solid_components(grid: &Grid) -> usize {
    let mut visited = vec![false; grid.cells.len()];
    let mut components = 0;
    for index in 0..grid.cells.len() {
        if grid.cells[index] != Cell::Solid || visited[index] {
            continue;
        }
        components += 1;
        flood(grid, index, Cell::Solid, Connectivity::Four, &mut visited);
    }
    components
}

fn background_hole_areas(grid: &Grid) -> Vec<usize> {
    let mut visited = vec![false; grid.cells.len()];
    let mut holes = Vec::new();
    for index in 0..grid.cells.len() {
        if grid.cells[index] != Cell::Background || visited[index] {
            continue;
        }
        let component = flood(
            grid,
            index,
            Cell::Background,
            Connectivity::Eight,
            &mut visited,
        );
        if !component.touches_border {
            holes.push(component.area);
        }
    }
    holes
}

#[derive(Clone, Copy)]
enum Connectivity {
    Four,
    Eight,
}

struct FloodResult {
    area: usize,
    touches_border: bool,
}

fn flood(
    grid: &Grid,
    start: usize,
    target: Cell,
    connectivity: Connectivity,
    visited: &mut [bool],
) -> FloodResult {
    let mut queue = VecDeque::from([start]);
    visited[start] = true;
    let mut area = 0;
    let mut touches_border = false;

    while let Some(index) = queue.pop_front() {
        area += 1;
        let row = index / grid.width;
        let column = index % grid.width;
        touches_border |= row == 0
            || column == 0
            || row + 1 == grid.height
            || column + 1 == grid.width;

        for (next_row, next_column) in neighbours(grid, row, column, connectivity) {
            let next = grid.index(next_row, next_column);
            if !visited[next] && grid.cells[next] == target {
                visited[next] = true;
                queue.push_back(next);
            }
        }
    }

    FloodResult {
        area,
        touches_border,
    }
}

fn neighbours(
    grid: &Grid,
    row: usize,
    column: usize,
    connectivity: Connectivity,
) -> impl Iterator<Item = (usize, usize)> {
    const FOUR: &[(isize, isize)] = &[(-1, 0), (0, -1), (0, 1), (1, 0)];
    const EIGHT: &[(isize, isize)] = &[
        (-1, -1),
        (-1, 0),
        (-1, 1),
        (0, -1),
        (0, 1),
        (1, -1),
        (1, 0),
        (1, 1),
    ];
    let offsets = match connectivity {
        Connectivity::Four => FOUR,
        Connectivity::Eight => EIGHT,
    };
    offsets.iter().filter_map(move |(row_delta, column_delta)| {
        let next_row = row.checked_add_signed(*row_delta)?;
        let next_column = column.checked_add_signed(*column_delta)?;
        (next_row < grid.height && next_column < grid.width)
            .then_some((next_row, next_column))
    })
}

fn error(message: impl Into<String>) -> ParseGridError {
    ParseGridError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::{Grid, Topology, analyze};

    fn topology(input: &str) -> Topology {
        analyze(&Grid::parse(input).expect("fixture should parse"))
    }

    #[test]
    fn rejects_ragged_and_unknown_input() {
        assert!(Grid::parse("##\n#").is_err());
        assert!(Grid::parse("#x").is_err());
    }

    #[test]
    fn solid_rectangle_has_no_holes() {
        let result = topology("###\n###\n");
        assert_eq!(result.solid_components, 1);
        assert_eq!(result.holes, 0);
        assert_eq!(result.euler_characteristic, 1);
    }

    #[test]
    fn connected_region_can_enclose_two_holes() {
        let result = topology("#########\n#...#...#\n#...#...#\n#...#...#\n#########\n");
        assert_eq!(result.solid_components, 1);
        assert_eq!(result.holes, 2);
        assert_eq!(result.hole_areas, vec![9, 9]);
        assert_eq!(result.euler_characteristic, -1);
    }

    #[test]
    fn diagonal_background_opening_is_exterior_under_dual_connectivity() {
        let result = topology(".###\n#.##\n##.#\n###.\n");
        assert_eq!(result.holes, 0);
    }

    #[test]
    fn island_inside_a_cavity_changes_euler_characteristic() {
        let result = topology("#######\n#.....#\n#..#..#\n#.....#\n#######\n");
        assert_eq!(result.solid_components, 2);
        assert_eq!(result.holes, 1);
        assert_eq!(result.euler_characteristic, 1);
    }
}
