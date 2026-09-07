use crate::clustering::cluster_items;
use crate::geometry::{objects_to_bbox, rect_to_edges, snap_edges};
use crate::text::{extract_text, extract_words, TextOptions};
use crate::types::{BBox, Char, Edge, Line, Orientation, Page, Word};
use ordered_float::OrderedFloat;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableStrategy {
    #[default]
    Lines,
    LinesStrict,
    Text,
    Explicit,
}

impl std::str::FromStr for TableStrategy {
    type Err = crate::Error;

    fn from_str(s: &str) -> crate::Result<Self> {
        match s {
            "lines" => Ok(Self::Lines),
            "lines_strict" => Ok(Self::LinesStrict),
            "text" => Ok(Self::Text),
            "explicit" => Ok(Self::Explicit),
            other => Err(crate::Error::Message(format!(
                "unknown table strategy: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub enum ExplicitLine {
    Position(f64),
    Edge(Edge),
}

#[derive(Debug, Clone)]
pub struct TableSettings {
    pub vertical_strategy: TableStrategy,
    pub horizontal_strategy: TableStrategy,
    pub explicit_vertical_lines: Vec<ExplicitLine>,
    pub explicit_horizontal_lines: Vec<ExplicitLine>,
    pub snap_tolerance: f64,
    pub snap_x_tolerance: Option<f64>,
    pub snap_y_tolerance: Option<f64>,
    pub join_tolerance: f64,
    pub join_x_tolerance: Option<f64>,
    pub join_y_tolerance: Option<f64>,
    pub edge_min_length: f64,
    pub edge_min_length_prefilter: f64,
    pub min_words_vertical: usize,
    pub min_words_horizontal: usize,
    pub intersection_tolerance: f64,
    pub intersection_x_tolerance: Option<f64>,
    pub intersection_y_tolerance: Option<f64>,
    pub text_options: TextOptions,
}

impl Default for TableSettings {
    fn default() -> Self {
        Self {
            vertical_strategy: TableStrategy::Lines,
            horizontal_strategy: TableStrategy::Lines,
            explicit_vertical_lines: Vec::new(),
            explicit_horizontal_lines: Vec::new(),
            snap_tolerance: 3.0,
            snap_x_tolerance: None,
            snap_y_tolerance: None,
            join_tolerance: 3.0,
            join_x_tolerance: None,
            join_y_tolerance: None,
            edge_min_length: 3.0,
            edge_min_length_prefilter: 1.0,
            min_words_vertical: 3,
            min_words_horizontal: 1,
            intersection_tolerance: 3.0,
            intersection_x_tolerance: None,
            intersection_y_tolerance: None,
            text_options: TextOptions::default(),
        }
    }
}

impl TableSettings {
    pub fn snap_x_tolerance(&self) -> f64 {
        self.snap_x_tolerance.unwrap_or(self.snap_tolerance)
    }

    pub fn snap_y_tolerance(&self) -> f64 {
        self.snap_y_tolerance.unwrap_or(self.snap_tolerance)
    }

    pub fn join_x_tolerance(&self) -> f64 {
        self.join_x_tolerance.unwrap_or(self.join_tolerance)
    }

    pub fn join_y_tolerance(&self) -> f64 {
        self.join_y_tolerance.unwrap_or(self.join_tolerance)
    }

    pub fn intersection_x_tolerance(&self) -> f64 {
        self.intersection_x_tolerance
            .unwrap_or(self.intersection_tolerance)
    }

    pub fn intersection_y_tolerance(&self) -> f64 {
        self.intersection_y_tolerance
            .unwrap_or(self.intersection_tolerance)
    }
}

pub fn table_rows_to_csv(rows: &[Vec<Option<String>>]) -> crate::Result<String> {
    let mut output = Vec::new();
    {
        let mut writer = csv::Writer::from_writer(&mut output);
        for row in rows {
            let record = row
                .iter()
                .map(|cell| cell.as_deref().unwrap_or_default())
                .collect::<Vec<_>>();
            writer.write_record(record)?;
        }
        writer.flush()?;
    }
    String::from_utf8(output).map_err(|err| crate::Error::Message(err.to_string()))
}

#[derive(Debug, Clone)]
pub struct CellGroup {
    pub cells: Vec<Option<BBox>>,
    pub bbox: BBox,
}

#[derive(Debug, Clone)]
pub struct Table {
    pub cells: Vec<BBox>,
    pub bbox: BBox,
}

impl Table {
    pub fn rows(&self) -> Vec<CellGroup> {
        self.get_rows_or_cols(true)
    }

    pub fn columns(&self) -> Vec<CellGroup> {
        self.get_rows_or_cols(false)
    }

    pub fn row_count(&self) -> usize {
        self.rows().len()
    }

    pub fn column_count(&self) -> usize {
        self.columns().len()
    }

    pub fn shape(&self) -> (usize, usize) {
        (self.row_count(), self.column_count())
    }

    pub fn extract_csv(&self, page: &Page, options: &TextOptions) -> crate::Result<String> {
        table_rows_to_csv(&self.extract(page, options))
    }

    pub fn extract(&self, page: &Page, options: &TextOptions) -> Vec<Vec<Option<String>>> {
        let mut table_arr = Vec::new();
        for row in self.rows() {
            let mut row_arr = Vec::new();
            let row_chars: Vec<Char> = page
                .chars
                .iter()
                .filter(|ch| char_in_bbox(ch, row.bbox))
                .cloned()
                .collect();

            for cell in row.cells {
                if let Some(cell_bbox) = cell {
                    let cell_chars: Vec<Char> = row_chars
                        .iter()
                        .filter(|ch| char_in_bbox(ch, cell_bbox))
                        .cloned()
                        .collect();
                    if cell_chars.is_empty() {
                        row_arr.push(Some(String::new()));
                    } else {
                        let mut text_options = options.clone();
                        if text_options.layout {
                            text_options.layout_width = Some(cell_bbox.width());
                            text_options.layout_height = Some(cell_bbox.height());
                            text_options.layout_bbox = Some(cell_bbox);
                        }
                        row_arr.push(Some(extract_text(&cell_chars, &text_options)));
                    }
                } else {
                    row_arr.push(None);
                }
            }
            table_arr.push(row_arr);
        }
        table_arr
    }

    fn get_rows_or_cols(&self, rows: bool) -> Vec<CellGroup> {
        let (axis, antiaxis) = if rows {
            (0usize, 1usize)
        } else {
            (1usize, 0usize)
        };

        let mut cells = self.cells.clone();
        cells.sort_by(|a, b| {
            let ka = bbox_coord(*a, antiaxis)
                .total_cmp(&bbox_coord(*b, antiaxis))
                .then_with(|| bbox_coord(*a, axis).total_cmp(&bbox_coord(*b, axis)));
            ka
        });

        let mut xs: Vec<f64> = cells.iter().map(|bbox| bbox_coord(*bbox, axis)).collect();
        xs.sort_by(|a, b| a.total_cmp(b));
        xs.dedup_by(|a, b| (*a - *b).abs() < 0.0001);

        let groups = cluster_items(&cells, |bbox| bbox_coord(*bbox, antiaxis), 0.0);
        let mut out = Vec::new();
        for group in groups {
            let mut row_cells = Vec::new();
            for x in &xs {
                let cell = group
                    .iter()
                    .find(|bbox| (bbox_coord(**bbox, axis) - *x).abs() < 0.0001)
                    .copied();
                row_cells.push(cell);
            }

            let bbox = merged_optional_bbox(&row_cells);
            out.push(CellGroup {
                cells: row_cells,
                bbox,
            });
        }
        out
    }
}

#[derive(Debug, Clone)]
pub struct TableFinder {
    pub settings: TableSettings,
    pub edges: Vec<Edge>,
    pub intersections: BTreeMap<PointKey, Intersection>,
    pub cells: Vec<BBox>,
    pub tables: Vec<Table>,
}

impl TableFinder {
    pub fn new(page: &Page, settings: TableSettings) -> crate::Result<Self> {
        let edges = get_edges(page, &settings)?;
        let intersections = edges_to_intersections(
            &edges,
            settings.intersection_x_tolerance(),
            settings.intersection_y_tolerance(),
        );
        let cells = intersections_to_cells(&intersections);
        let tables = cells_to_tables(&cells)
            .into_iter()
            .map(|group| {
                let bbox = merge_bboxes(&group);
                Table { cells: group, bbox }
            })
            .collect();

        Ok(Self {
            settings,
            edges,
            intersections,
            cells,
            tables,
        })
    }
}

type PointKey = (OrderedFloat<f64>, OrderedFloat<f64>);
type BBoxKey = (
    OrderedFloat<f64>,
    OrderedFloat<f64>,
    OrderedFloat<f64>,
    OrderedFloat<f64>,
);

#[derive(Debug, Clone, Default)]
pub struct Intersection {
    pub vertical: Vec<Edge>,
    pub horizontal: Vec<Edge>,
}

fn get_edges(page: &Page, settings: &TableSettings) -> crate::Result<Vec<Edge>> {
    if matches!(settings.vertical_strategy, TableStrategy::Explicit)
        && settings.explicit_vertical_lines.len() < 2
    {
        return Err(crate::Error::Message(
            "explicit vertical strategy requires at least two explicit vertical lines".to_string(),
        ));
    }
    if matches!(settings.horizontal_strategy, TableStrategy::Explicit)
        && settings.explicit_horizontal_lines.len() < 2
    {
        return Err(crate::Error::Message(
            "explicit horizontal strategy requires at least two explicit horizontal lines"
                .to_string(),
        ));
    }

    let words = if matches!(settings.vertical_strategy, TableStrategy::Text)
        || matches!(settings.horizontal_strategy, TableStrategy::Text)
    {
        extract_words(&page.chars, &settings.text_options, false)
    } else {
        Vec::new()
    };

    let mut v_explicit = Vec::new();
    for entry in &settings.explicit_vertical_lines {
        match entry {
            ExplicitLine::Position(x) => v_explicit.push(Edge {
                x0: *x,
                top: page.bbox.top,
                x1: *x,
                bottom: page.bbox.bottom,
                width: 0.0,
                height: page.bbox.height(),
                orientation: Orientation::Vertical,
                object_type: "explicit".to_string(),
            }),
            ExplicitLine::Edge(edge) => {
                if edge.orientation == Orientation::Vertical {
                    v_explicit.push(edge.clone());
                }
            }
        }
    }

    let mut h_explicit = Vec::new();
    for entry in &settings.explicit_horizontal_lines {
        match entry {
            ExplicitLine::Position(y) => h_explicit.push(Edge {
                x0: page.bbox.x0,
                top: *y,
                x1: page.bbox.x1,
                bottom: *y,
                width: page.bbox.width(),
                height: 0.0,
                orientation: Orientation::Horizontal,
                object_type: "explicit".to_string(),
            }),
            ExplicitLine::Edge(edge) => {
                if edge.orientation == Orientation::Horizontal {
                    h_explicit.push(edge.clone());
                }
            }
        }
    }

    let mut vertical = match settings.vertical_strategy {
        TableStrategy::Lines => filter_edges(
            &page.edges(),
            Some(Orientation::Vertical),
            None,
            settings.edge_min_length_prefilter,
        ),
        TableStrategy::LinesStrict => filter_edges(
            &page.edges(),
            Some(Orientation::Vertical),
            Some("line"),
            settings.edge_min_length_prefilter,
        ),
        TableStrategy::Text => words_to_edges_v(&words, settings.min_words_vertical),
        TableStrategy::Explicit => Vec::new(),
    };
    vertical.extend(v_explicit);

    let mut horizontal = match settings.horizontal_strategy {
        TableStrategy::Lines => filter_edges(
            &page.edges(),
            Some(Orientation::Horizontal),
            None,
            settings.edge_min_length_prefilter,
        ),
        TableStrategy::LinesStrict => filter_edges(
            &page.edges(),
            Some(Orientation::Horizontal),
            Some("line"),
            settings.edge_min_length_prefilter,
        ),
        TableStrategy::Text => words_to_edges_h(&words, settings.min_words_horizontal),
        TableStrategy::Explicit => Vec::new(),
    };
    horizontal.extend(h_explicit);

    let mut edges = Vec::new();
    edges.extend(vertical);
    edges.extend(horizontal);

    let merged = merge_edges(
        &edges,
        settings.snap_x_tolerance(),
        settings.snap_y_tolerance(),
        settings.join_x_tolerance(),
        settings.join_y_tolerance(),
    );

    Ok(filter_edges(&merged, None, None, settings.edge_min_length))
}

fn filter_edges(
    edges: &[Edge],
    orientation: Option<Orientation>,
    edge_type: Option<&str>,
    min_length: f64,
) -> Vec<Edge> {
    edges
        .iter()
        .filter(|edge| {
            let orientation_ok = orientation
                .map(|value| value == edge.orientation)
                .unwrap_or(true);
            let edge_type_ok = edge_type
                .map(|value| value == edge.object_type)
                .unwrap_or(true);
            let dim = if edge.orientation == Orientation::Vertical {
                edge.height
            } else {
                edge.width
            };
            orientation_ok && edge_type_ok && dim >= min_length
        })
        .cloned()
        .collect()
}

fn merge_edges(
    edges: &[Edge],
    snap_x_tolerance: f64,
    snap_y_tolerance: f64,
    join_x_tolerance: f64,
    join_y_tolerance: f64,
) -> Vec<Edge> {
    let snapped = if snap_x_tolerance > 0.0 || snap_y_tolerance > 0.0 {
        snap_edges(edges, snap_x_tolerance, snap_y_tolerance)
    } else {
        edges.to_vec()
    };

    let mut sorted = snapped;
    sorted.sort_by(|a, b| edge_group_key(a).cmp(&edge_group_key(b)));

    let mut out = Vec::new();
    let mut idx = 0usize;
    while idx < sorted.len() {
        let key = edge_group_key(&sorted[idx]);
        let mut end = idx + 1;
        while end < sorted.len() && edge_group_key(&sorted[end]) == key {
            end += 1;
        }

        let tolerance = if key.0 == 'h' {
            join_x_tolerance
        } else {
            join_y_tolerance
        };

        out.extend(join_edge_group(&sorted[idx..end], key.0, tolerance));
        idx = end;
    }

    out
}

fn edge_group_key(edge: &Edge) -> (char, OrderedFloat<f64>) {
    match edge.orientation {
        Orientation::Horizontal => ('h', OrderedFloat(edge.top)),
        Orientation::Vertical => ('v', OrderedFloat(edge.x0)),
    }
}

fn join_edge_group(edges: &[Edge], orientation: char, tolerance: f64) -> Vec<Edge> {
    if edges.is_empty() {
        return Vec::new();
    }
    let mut sorted = edges.to_vec();
    if orientation == 'h' {
        sorted.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    } else {
        sorted.sort_by(|a, b| a.top.total_cmp(&b.top));
    }

    let mut joined = vec![sorted[0].clone()];
    for edge in sorted.into_iter().skip(1) {
        let last = joined.last_mut().expect("non-empty");
        let overlaps = if orientation == 'h' {
            edge.x0 <= last.x1 + tolerance
        } else {
            edge.top <= last.bottom + tolerance
        };

        if overlaps {
            if orientation == 'h' && edge.x1 > last.x1 {
                last.x1 = edge.x1;
                last.width = last.x1 - last.x0;
            } else if orientation == 'v' && edge.bottom > last.bottom {
                last.bottom = edge.bottom;
                last.height = last.bottom - last.top;
            }
        } else {
            joined.push(edge);
        }
    }
    joined
}

fn words_to_edges_h(words: &[Word], threshold: usize) -> Vec<Edge> {
    let clusters = cluster_items(words, |word| word.top, 1.0);
    let large: Vec<Vec<Word>> = clusters
        .into_iter()
        .filter(|cluster| cluster.len() >= threshold)
        .collect();
    if large.is_empty() {
        return Vec::new();
    }

    let rects: Vec<BBox> = large
        .iter()
        .filter_map(|cluster| objects_to_bbox(cluster))
        .collect();

    if rects.is_empty() {
        return Vec::new();
    }

    let min_x0 = rects
        .iter()
        .map(|bbox| bbox.x0)
        .fold(f64::INFINITY, f64::min);
    let max_x1 = rects
        .iter()
        .map(|bbox| bbox.x1)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut edges = Vec::new();
    for rect in rects {
        edges.push(Edge {
            x0: min_x0,
            top: rect.top,
            x1: max_x1,
            bottom: rect.top,
            width: max_x1 - min_x0,
            height: 0.0,
            orientation: Orientation::Horizontal,
            object_type: "text".to_string(),
        });
        edges.push(Edge {
            x0: min_x0,
            top: rect.bottom,
            x1: max_x1,
            bottom: rect.bottom,
            width: max_x1 - min_x0,
            height: 0.0,
            orientation: Orientation::Horizontal,
            object_type: "text".to_string(),
        });
    }
    edges
}

fn words_to_edges_v(words: &[Word], threshold: usize) -> Vec<Edge> {
    let by_x0 = cluster_items(words, |word| word.x0, 1.0);
    let by_x1 = cluster_items(words, |word| word.x1, 1.0);
    let by_center = cluster_items(words, |word| (word.x0 + word.x1) / 2.0, 1.0);

    let mut clusters = Vec::new();
    clusters.extend(by_x0);
    clusters.extend(by_x1);
    clusters.extend(by_center);
    clusters.sort_by(|a, b| b.len().cmp(&a.len()));

    let large: Vec<Vec<Word>> = clusters
        .into_iter()
        .filter(|cluster| cluster.len() >= threshold)
        .collect();
    let mut boxes: Vec<BBox> = large
        .iter()
        .filter_map(|cluster| objects_to_bbox(cluster))
        .collect();

    let mut condensed = Vec::new();
    for bbox in boxes.drain(..) {
        if !condensed
            .iter()
            .any(|existing: &BBox| existing.overlap(bbox).is_some())
        {
            condensed.push(bbox);
        }
    }

    if condensed.is_empty() {
        return Vec::new();
    }

    condensed.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    let max_x1 = condensed
        .iter()
        .map(|bbox| bbox.x1)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_top = condensed
        .iter()
        .map(|bbox| bbox.top)
        .fold(f64::INFINITY, f64::min);
    let max_bottom = condensed
        .iter()
        .map(|bbox| bbox.bottom)
        .fold(f64::NEG_INFINITY, f64::max);

    let mut out = Vec::new();
    for bbox in &condensed {
        out.push(Edge {
            x0: bbox.x0,
            top: min_top,
            x1: bbox.x0,
            bottom: max_bottom,
            width: 0.0,
            height: max_bottom - min_top,
            orientation: Orientation::Vertical,
            object_type: "text".to_string(),
        });
    }

    out.push(Edge {
        x0: max_x1,
        top: min_top,
        x1: max_x1,
        bottom: max_bottom,
        width: 0.0,
        height: max_bottom - min_top,
        orientation: Orientation::Vertical,
        object_type: "text".to_string(),
    });

    out
}

fn edges_to_intersections(
    edges: &[Edge],
    x_tolerance: f64,
    y_tolerance: f64,
) -> BTreeMap<PointKey, Intersection> {
    let vertical: Vec<Edge> = edges
        .iter()
        .filter(|edge| edge.orientation == Orientation::Vertical)
        .cloned()
        .collect();
    let horizontal: Vec<Edge> = edges
        .iter()
        .filter(|edge| edge.orientation == Orientation::Horizontal)
        .cloned()
        .collect();

    let mut intersections: BTreeMap<PointKey, Intersection> = BTreeMap::new();
    for v in &vertical {
        for h in &horizontal {
            if v.top <= h.top + y_tolerance
                && v.bottom >= h.top - y_tolerance
                && v.x0 >= h.x0 - x_tolerance
                && v.x0 <= h.x1 + x_tolerance
            {
                let key = (OrderedFloat(v.x0), OrderedFloat(h.top));
                let entry = intersections.entry(key).or_default();
                entry.vertical.push(v.clone());
                entry.horizontal.push(h.clone());
            }
        }
    }
    intersections
}

fn intersections_to_cells(intersections: &BTreeMap<PointKey, Intersection>) -> Vec<BBox> {
    let points: Vec<PointKey> = intersections.keys().copied().collect();
    let mut out = Vec::new();

    for (idx, point) in points.iter().enumerate() {
        let below: Vec<PointKey> = points
            .iter()
            .copied()
            .skip(idx + 1)
            .filter(|other| other.0 == point.0)
            .collect();
        let right: Vec<PointKey> = points
            .iter()
            .copied()
            .skip(idx + 1)
            .filter(|other| other.1 == point.1)
            .collect();

        for below_pt in &below {
            if !edge_connects(*point, *below_pt, intersections) {
                continue;
            }
            for right_pt in &right {
                if !edge_connects(*point, *right_pt, intersections) {
                    continue;
                }

                let bottom_right = (right_pt.0, below_pt.1);
                if intersections.contains_key(&bottom_right)
                    && edge_connects(bottom_right, *right_pt, intersections)
                    && edge_connects(bottom_right, *below_pt, intersections)
                {
                    out.push(BBox::new(
                        point.0.into_inner(),
                        point.1.into_inner(),
                        right_pt.0.into_inner(),
                        below_pt.1.into_inner(),
                    ));
                    break;
                }
            }
        }
    }

    out
}

fn edge_connects(
    p1: PointKey,
    p2: PointKey,
    intersections: &BTreeMap<PointKey, Intersection>,
) -> bool {
    if p1.0 == p2.0 {
        let a: BTreeSet<BBoxKey> = intersections[&p1]
            .vertical
            .iter()
            .map(edge_bbox_key)
            .collect();
        let b: BTreeSet<BBoxKey> = intersections[&p2]
            .vertical
            .iter()
            .map(edge_bbox_key)
            .collect();
        return !a.is_disjoint(&b);
    }

    if p1.1 == p2.1 {
        let a: BTreeSet<BBoxKey> = intersections[&p1]
            .horizontal
            .iter()
            .map(edge_bbox_key)
            .collect();
        let b: BTreeSet<BBoxKey> = intersections[&p2]
            .horizontal
            .iter()
            .map(edge_bbox_key)
            .collect();
        return !a.is_disjoint(&b);
    }

    false
}

fn cells_to_tables(cells: &[BBox]) -> Vec<Vec<BBox>> {
    let mut remaining = cells.to_vec();
    let mut current_corners: BTreeSet<PointKey> = BTreeSet::new();
    let mut current_cells: Vec<BBox> = Vec::new();
    let mut tables = Vec::new();

    while !remaining.is_empty() {
        let initial = current_cells.len();

        let snapshot = remaining.clone();
        for cell in snapshot {
            let corners = bbox_corners(cell);
            if current_cells.is_empty() {
                current_corners.extend(corners);
                current_cells.push(cell);
                remove_bbox(&mut remaining, cell);
            } else {
                let corner_count = corners
                    .iter()
                    .filter(|corner| current_corners.contains(corner))
                    .count();
                if corner_count > 0 {
                    current_corners.extend(corners);
                    current_cells.push(cell);
                    remove_bbox(&mut remaining, cell);
                }
            }
        }

        if current_cells.len() == initial {
            tables.push(current_cells.clone());
            current_corners.clear();
            current_cells.clear();
        }
    }

    if !current_cells.is_empty() {
        tables.push(current_cells);
    }

    tables.retain(|table| table.len() > 1);
    tables.sort_by(|a, b| {
        let aa = top_left_of_table(a);
        let bb = top_left_of_table(b);
        aa.0.total_cmp(&bb.0).then_with(|| aa.1.total_cmp(&bb.1))
    });
    tables
}

fn char_in_bbox(ch: &Char, bbox: BBox) -> bool {
    let bbox = bbox.normalized();
    let v_mid = (ch.top + ch.bottom) / 2.0;
    let h_mid = (ch.x0 + ch.x1) / 2.0;
    h_mid >= bbox.x0 && h_mid < bbox.x1 && v_mid >= bbox.top && v_mid < bbox.bottom
}

fn merge_bboxes(boxes: &[BBox]) -> BBox {
    boxes
        .iter()
        .copied()
        .map(BBox::normalized)
        .reduce(|acc, bbox| acc.union(bbox))
        .unwrap_or_default()
}

fn merged_optional_bbox(boxes: &[Option<BBox>]) -> BBox {
    let selected: Vec<BBox> = boxes.iter().filter_map(|bbox| *bbox).collect();
    merge_bboxes(&selected)
}

fn bbox_coord(bbox: BBox, axis: usize) -> f64 {
    match axis {
        0 => bbox.x0,
        1 => bbox.top,
        _ => unreachable!(),
    }
}

fn bbox_corners(bbox: BBox) -> [PointKey; 4] {
    [
        (OrderedFloat(bbox.x0), OrderedFloat(bbox.top)),
        (OrderedFloat(bbox.x0), OrderedFloat(bbox.bottom)),
        (OrderedFloat(bbox.x1), OrderedFloat(bbox.top)),
        (OrderedFloat(bbox.x1), OrderedFloat(bbox.bottom)),
    ]
}

fn top_left_of_table(cells: &[BBox]) -> (f64, f64) {
    let bbox = merge_bboxes(cells);
    (bbox.top, bbox.x0)
}

fn edge_bbox_key(edge: &Edge) -> BBoxKey {
    (
        OrderedFloat(edge.x0),
        OrderedFloat(edge.top),
        OrderedFloat(edge.x1),
        OrderedFloat(edge.bottom),
    )
}

fn remove_bbox(items: &mut Vec<BBox>, target: BBox) {
    if let Some(idx) = items.iter().position(|bbox| *bbox == target) {
        items.remove(idx);
    }
}

fn line_to_edge(line: &Line) -> Edge {
    let orientation = if (line.top - line.bottom).abs() < 0.0001 {
        Orientation::Horizontal
    } else {
        Orientation::Vertical
    };
    Edge {
        x0: line.x0,
        top: line.top,
        x1: line.x1,
        bottom: line.bottom,
        width: line.width,
        height: line.height,
        orientation,
        object_type: "line".to_string(),
    }
}

fn curve_to_edges(curve: &crate::types::Curve) -> Vec<Edge> {
    let mut edges = Vec::new();
    for pair in curve.pts.windows(2) {
        let p0 = pair[0];
        let p1 = pair[1];
        let orientation = if (p0.x - p1.x).abs() < 0.0001 {
            Some(Orientation::Vertical)
        } else if (p0.y - p1.y).abs() < 0.0001 {
            Some(Orientation::Horizontal)
        } else {
            None
        };

        if let Some(orientation) = orientation {
            edges.push(Edge {
                x0: p0.x.min(p1.x),
                top: p0.y.min(p1.y),
                x1: p0.x.max(p1.x),
                bottom: p0.y.max(p1.y),
                width: (p1.x - p0.x).abs(),
                height: (p1.y - p0.y).abs(),
                orientation,
                object_type: "curve_edge".to_string(),
            });
        }
    }
    edges
}

impl Page {
    pub fn edges(&self) -> Vec<Edge> {
        let mut edges: Vec<Edge> = self.lines.iter().map(line_to_edge).collect();
        for rect in &self.rects {
            edges.extend(rect_to_edges(rect));
        }
        for curve in &self.curves {
            edges.extend(curve_to_edges(curve));
        }
        edges
    }
}
