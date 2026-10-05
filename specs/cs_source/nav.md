# Counter-Strike: Source: navigation meshes (`maps/<map>.nav`)

Source basis: Valve's public Source SDK 2013. I read the shared server navigation-mesh code: file load/save, area, ladder, hiding-spot and place-directory records, area geometry helpers (height at a point, portals, containment, nearest-area search, spatial grid), the generic A* search with its stock "shortest path" cost, and the NextBot path builder and path follower (TF2/HL2MP bot framework). I also read the HL2MP bot's path cost as an example of a richer cost. CS:S's own bot code (the CS bot and its CS-specific nav-area subclass) is **not** in the SDK, so everything about how CS:S bots choose and follow paths is inferred or listed under Open questions. The format facts were checked against all 18 stock `.nav` files shipped with the Linux CS:S install: a throwaway parser written from this spec decodes every file to exactly its last byte.
Status: draft

## Summary

- A `.nav` file is a little-endian binary list of **areas**. Each area is an axis-aligned rectangle in XY with a height at each of its 4 corners, so it can be sloped or twisted. Each area keeps one-way links to neighbours, listed per side (N/E/S/W). Ladders are separate records that link a bottom area to up to four top areas.
- Pathfinding is A* over areas. Each step costs the distance between area centres, and more if the destination area is marked crouch or jump. A path follower then turns the area sequence into world points: portal crossings, drop-down and climb-up markers, ladder mount points.
- **CS:S ships two format versions.** de_dust, de_dust2 and de_inferno are version 16 (sub-version 1). The other 15 stock maps are version 9 (no sub-version). A loader must support both.

## Units and conventions

- Distances are Hammer units (1 unit = 1 inch). Z is up. Coordinates are world space, the same frame as the BSP.
- **Compass directions in the nav system:** North = −Y, East = +X, South = +Y, West = −X. This is not "north = +Y". Direction indices: N=0, E=1, S=2, W=3. Opposite: (d+2) mod 4.
- An area is stored as two explicit corners: **NW** = (min X, min Y, z) and **SE** = (max X, max Y, z). The other two corners are implicit. **NE** = (SE.x, NW.y) and **SW** = (NW.x, SE.y). Their heights are stored separately.
- Corner indices, where used: NW=0, NE=1, SE=2, SW=3.
- All multi-byte values are little-endian. `f32` is IEEE-754 single precision.
- Nothing in this spec depends on tick rate. Times stored in the file are seconds.

## Constants

| Name (ours) | Value | Unit | Meaning |
|---|---|---|---|
| magic | 0xFEEDFACE | u32 | first 4 bytes (`CE FA ED FE` on disk) |
| max_version | 16 | - | newest version the SDK code reads |
| generation_step | 25 | units | nav generator grid. Most area edges are multiples of 25 (837 of 961 on dust2). |
| step_height | 18 | units | climbable without jumping |
| jump_height | 41.8 | units | plain jump reach (nav analysis) |
| jump_crouch_height | 58 | units | crouch-jump reach, CS:S value (other games 64) |
| climb_up_height | 58 | units | CS:S: equal to jump_crouch_height |
| death_drop | 200 | units | CS:S: drop considered fatal (other games 400) |
| cliff_height | 300 | units | drop that flags an area as next to a cliff |
| half_human_width | 16 | units | |
| half_human_height | 35.5 | units | |
| human_height | 71 | units | |
| human_eye_height | 62 | units | |
| human_crouch_height | 55 | units | |
| human_crouch_eye_height | 37 | units | |
| grid_cell | 300 | units | spatial lookup cell size |
| beneath_limit | 120 | units | default: how far below a query point an area may be and still "contain" it |
| crouch_penalty | 20 | × dist | extra cost for entering a crouch area (stock cost) |
| jump_penalty | 5 | × dist | extra cost for entering a jump area (stock cost) |
| occupy_default | 120 | s | "earliest occupy time" when not computed |
| occupy_speed | 240 | units/s | speed used to turn path length into earliest-occupy time |
| max_path_segments | 256 | - | NextBot path node cap |
| goal_tolerance | 25 | units | NextBot: 2D distance at which a path node counts as reached |

## File layout

Notation: `u8/u16/u32/i32/f32`, `vec3` = 3×f32 (x, y, z). Conditions refer to the file's `version` (V) and `subversion` (S).

### Header

| Field | Type | Present when | Meaning |
|---|---|---|---|
| magic | u32 | always | must be 0xFEEDFACE |
| version | u32 | always | CS:S stock files: 9 or 16. Reject if > 16. |
| subversion | u32 | V ≥ 10 | owned by the game-specific subclass. CS:S writes **1**. |
| bsp_size | u32 | V ≥ 4 | byte size of the `.bsp` the mesh was built for. A mismatch only produces a "mesh out of date" warning. Loading continues. |
| analyzed | u8 | V ≥ 14 | non-zero = mesh marked as analysed |
| place_count | u16 | V ≥ 5 | number of place names |
| places[place_count] | see below | V ≥ 5 | place directory |
| has_unnamed_areas | u8 | V ≥ 12 | non-zero if some area has no place |
| (game pre-area data) | - | - | hook for subclasses. **CS:S writes nothing here** (verified). |
| area_count | u32 | always | 0 = invalid file |
| areas[area_count] | see below | | |
| ladder_count | u32 | V ≥ 6 | |
| ladders[ladder_count] | see below | V ≥ 6 | |
| (game mesh data) | - | - | hook for subclasses. **CS:S writes nothing here** (verified). The file ends right after the ladders. |

Place entry: `u16 len`, then `len` bytes, which are the name **including** its terminating NUL. Names are ASCII, for example `BombsiteA`. An area refers to a place by **1-based index** into this list. 0 means "no place". (The engine maps names to its own global place table at load. We only need the names.)

### Area record

| Field | Type | Present when | Meaning |
|---|---|---|---|
| id | u32 | always | unique, non-zero. In stock files ids ascend but have gaps (dust2: 2…4181 for 961 areas). |
| flags | u8 if V ≤ 8; u16 if 9 ≤ V ≤ 12; i32/u32 if V ≥ 13 | always | attribute bits, see Attribute flags |
| nw | vec3 | always | NW corner (min x, min y, z at NW) |
| se | vec3 | always | SE corner (max x, max y, z at SE) |
| ne_z | f32 | always | height of the implicit NE corner (se.x, nw.y) |
| sw_z | f32 | always | height of the implicit SW corner (nw.x, se.y) |
| connections | 4 × { u32 count; u32 area_id[count] } | always | in side order N, E, S, W |
| hiding_spot_count | u8 | always | |
| hiding_spots | see below | always | V = 1: bare vec3 each. V ≥ 2: record below. |
| approach_count | u8 | V < 15 | |
| approach[approach_count] | 14 bytes each: u32, u32, u8, u32, u8 | V < 15 | see Approach areas |
| encounter_count | u32 | always | V < 3 uses an obsolete layout. Not needed for CS:S. |
| encounters | see below | V ≥ 3 | |
| place | u16 | V ≥ 5 | 1-based index into the place list, 0 = none |
| ladder_links | 2 × { u32 count; u32 ladder_id[count] } | V ≥ 7 | list 0 = ladders going **up** from this area, list 1 = ladders going **down** |
| earliest_occupy | 2 × f32 | V ≥ 8 | seconds. Index = team number mod 2: [0] = Terrorists (team 2), [1] = CTs (team 3). |
| light | 4 × f32 | V ≥ 11 | light intensity 0..1 at NW, NE, SE, SW |
| visible_count | u32 | V ≥ 16 | |
| visible[visible_count] | { u32 area_id; u8 vis } | V ≥ 16 | potentially visible areas. vis bit 0x01 = partly visible, 0x02 = fully visible. Values 0..3 all occur. |
| inherit_visibility_from | u32 | V ≥ 16 | area id, or 0 |
| CS custom area data | u8 count (+ entries) | S ≥ 1 (CS:S subclass) | in all three CS:S v16 files this is one byte, **always 0**. See Open questions. Our parser: read a u8 count. If it is non-zero, assume 14-byte entries laid out like the approach records. |

Hiding spot record (V ≥ 2), 17 bytes: `u32 id; vec3 pos; u8 flags`. Flags: 0x01 in cover, 0x02 good sniper spot, 0x04 ideal sniper spot, 0x08 exposed. `pos` is on the ground (z equals the area's surface). Spot ids are global across the mesh. On dust2 they run 0..535 with no gaps. Each spot belongs to the area whose record lists it.

Encounter record (V ≥ 3): `u32 from_area; u8 from_dir; u32 to_area; u8 to_dir; u8 spot_count; spot_count × { u32 hiding_spot_id; u8 t }`. Fraction = t / 255. Meaning: a bot that enters from `from_area` across that area's `from_dir` side, and leaves toward `to_area` across that area's `to_dir` side, walks a straight line between the two portal centres, raised 35.5 units above the floor. The listed hiding spots become visible along that line, in order, at parametric position t (0..1). Bots use this for "check corners as you walk". It is not needed for pathing. **The CS:S v16 files contain no encounters at all.**

Approach record (V < 15), 14 bytes, 5 fields: `u32 area_a; u32 area_b; u8 how_ab; u32 area_c; u8 how_bc`. The SDK reads and discards these. The usual reading is: approach area, the area before it on the route, how it was entered, the area after it, how it is left. "how" uses the traverse codes below. This is not confirmed from source (Open questions). Observed on de_train area #2: (919, 922, 1, 920, 1).

Traverse ("how") codes: 0..3 = walk across side N/E/S/W, 4 = ladder up, 5 = ladder down, 6 = jump, 7 = elevator up, 8 = elevator down.

### Ladder record

V ≥ 7: 56 bytes. V = 6 has one extra byte after `dir` (an obsolete "dangling" bool).

| Field | Type | Meaning |
|---|---|---|
| id | u32 | ladder id, referenced from area ladder_links |
| width | f32 | ladder width, units |
| top | vec3 | top end of the climbable line |
| bottom | vec3 | bottom end |
| length | f32 | ladder length used as path cost. Usually \|top − bottom\|, but **not always**: de_train ladder 2 stores 94 while \|top−bottom\| = 81.86. Use the stored value. |
| dir | u32 | compass direction (0..3) the climbable face points to (its outward normal). The climber stands on that side, facing the opposite way. |
| top_forward_area | u32 | area reached by climbing over the top and continuing straight on (it lies in the opposite direction to `dir`). 0 = none. |
| top_left_area | u32 | area off the top to one side (see below). 0 = none. |
| top_right_area | u32 | area off the top to the other side. 0 = none. |
| top_behind_area | u32 | area at the top on the climber's back side (direction `dir`). 0 = none. Only useful when going down. |
| bottom_area | u32 | area at the foot. 0 = none (broken ladder). |

When connections are made, "left" is the area in direction `rotate_left(dir)` and "right" the remaining side. `rotate_left` maps N→W, W→S, S→E, E→N, and `rotate_right` is the inverse. Ladder normal: the unit vector of `dir` (N = (0,−1,0), E = (1,0,0), S = (0,1,0), W = (−1,0,0)). The engine refines it at load with a world trace against the ladder surface. Using the compass vector is fine for us.

## Behavior

### Derived per-area values (computed at load)

- `center = ((nw + se) / 2)` in all three axes. **z is the mean of the NW and SE heights only**, not the surface height at the centre (see Quirks).
- `inv_dx = 1/(se.x − nw.x)`, `inv_dy = 1/(se.y − nw.y)`. If either size is ≤ 0 the area is degenerate: both become 0 (none in dust2).
- A connection whose target id equals the area's own id is dropped at load. Duplicate ladder ids in one direction list are dropped.
- Connection length = |center(target) − center(self)|, a 3D distance. It is used as the step cost.
- Ids are resolved to references after all areas are read. An unknown non-zero id makes the mesh "corrupt", but loading still finishes.
- Incoming one-way links: after load, for every link A→B on side d where B has no link back to A on side opposite(d), B records A as an "incoming" neighbour on side opposite(d). This is only used by tools and AI queries, not by A*.
- The engine also re-tests every area for stairs at load with world traces (see Quirks). We can skip this and trust the stored flag.

### Height of an area at (x, y)

```
if inv_dx == 0 or inv_dy == 0: return ne_z
u = clamp((x − nw.x) · inv_dx, 0, 1)
v = clamp((y − nw.y) · inv_dy, 0, 1)
north = nw.z + u · (ne_z − nw.z)        # along the north (min-y) edge
south = sw_z + u · (se.z − sw_z)        # along the south (max-y) edge
z     = north + v · (south − north)
```

This is bilinear interpolation, and points outside the rectangle are clamped onto it. The four corners need not be coplanar.

Closest point on an area to p: clamp p.x to [nw.x, se.x] and p.y to [nw.y, se.y], then z = height at that (x, y).

Area surface normal (used to measure drops): u = (se.x−nw.x, 0, ne_z−nw.z), v = (0, se.y−nw.y, sw_z−nw.z), n = normalize(u × v). n.z > 0 for a valid area.

### Overlap and containment

- **2D overlap** of point p with tolerance t: `nw.x ≤ p.x + t`, `p.x − t ≤ se.x`, and the same for y. All are inclusive, so shared edges count for both areas.
- **Area at position** (the main "which area am I in" query, beneath_limit = 120 by default):
  1. Let `test_z = p.z + 5`.
  2. Over all areas that overlap (p.x, p.y) in 2D: compute `z = height(p.x, p.y)`. Skip the area if `z > test_z` (it is above us). Skip it if `z < p.z − beneath_limit`.
  3. Return the area with the highest z. On an exact tie the earlier-found area wins, which is file order within the grid cell.
  4. If no area passes, the result is none.
- **Area for an actor** (a variant used for bots and players): keep the actor's last known area if it still overlaps in 2D and its height is within ±18 (step_height) of the feet z. Otherwise run the query above, allowing areas up to 18 above the feet instead of 5, and skipping areas blocked for the actor's team.
- **"Contains"** (used to detect the goal during A* when there is no goal area): p overlaps in 2D, the area height at p is no more than 18 above p.z, and no other area that overlaps p in 2D lies between them (an area whose height at p is ≤ p.z and above this area's height at p).

### Nearest area (fallback when off the mesh)

Inputs: position p, max_dist (default 10000), check_los (default false), check_ground (default true), team.

1. If neither LOS nor ground checks are requested, try "area at position" first and return its result if it finds one. With the defaults (check_ground = true) this shortcut is **skipped**.
2. Source point: s.xy = p.xy. s.z = ground height below p (see below) + 35.5. If no ground is found: if check_ground, return none; otherwise use p.z + 35.5.
3. For each area (search rings of grid cells outward from p's cell, any order is fine): skip areas blocked for the team. Compute `c` = the closest point on the area to s. Use `d² = |c − p|²`. **Note: distance is to the original p, not to s** (see Open questions). Keep the area with the smallest d² that is below max_dist².
4. With check_los, also reject areas without clear line of sight: trace from p (nudged up out of solid) to above c, and from c up to p's height when the height difference exceeds 18.
5. Once a candidate is found, the ring search goes one ring further and then stops. That is only an optimisation, so a brute-force search gives the same answer.

Ground height below p (a world raycast): cast down from p.z + 35.5. Accept the first hit that lies at least 35.5 below the cast start, i.e. at or below p.z. If the hit is closer than that (a surface between p.z and p.z + 35.5), restart the cast from 35.5 above that surface. Give up (no ground) once the restart surface is 100 or more above p.z. Doors, breakables and toggle brushes are ignored by this trace.

### Spatial grid (acceleration only)

Grid origin = (min nw.x, min nw.y) over all areas. Cells are 300 × 300. Cell count per axis = floor((max − min) / 300) + 1. A world coordinate maps to `floor((w − min) / 300)` clamped to [0, count−1]. Each area is entered into every cell its rectangle touches. On dust2 the grid is 14 × 14. Any spatial index that returns the same candidates works. The only observable effect of this one is the tie-break order in "area at position".

### Connections and traversal semantics

- Connections are **directed and per side.** A→B listed under A's side d means "from A you can walk across A's side d into B". The reverse link is a separate entry in B's list, normally on side opposite(d). Missing reverse links are deliberate one-way drops (dust2: 64 of 3114 links are one-way). In 2 dust2 cases the reverse link sits on a non-opposite side. Treat links purely as directed edges and keep the side for portal maths.
- The side d says which edge of A the bot leaves through. B is usually directly across that edge, but may be lower (drop-down), higher (step or jump up) or separated by a gap (jump across). Nothing in the file says which. Path building works it out from geometry (see Path building).
- **Ladder links**: an id in A's "up" list means "from A you can climb ladder L up". The destinations are L's top_forward, top_left and top_right areas (**never** top_behind). An id in A's "down" list means "from A you can climb down L to L.bottom_area".
- Elevators are runtime-only (from entities). CS:S stock maps have none, so we skip them.

### Portal between two linked areas

For a link from A across A's side d to B, call it portal(A, B, d):

- d = N or S: the portal line lies on A's edge, at y = nw.y (N) or se.y (S). The x range is [max(A.nw.x, B.nw.x), min(A.se.x, B.se.x)], with both ends clamped into [A.nw.x, A.se.x]. center.x = the midpoint of that range, half_width = half its length.
- d = E or W: the same, with x = se.x (E) or nw.x (W), and the y range computed from the y extents.
- center.z = A's height at the centre.

**Closest crossing point** from position f, for path building: the same line, but the range ends are not clamped to A. If B's side lies on the mesh edge, the range is pulled in by a 25-unit margin:
- For N/S portals: the left end moves +25 if B has no two-way link on its West side, and the right end moves −25 if B has no two-way link on its East side.
- For E/W portals the same applies with B's North (top end +25) and South (bottom end −25) sides.
- "No two-way link on side s" means none of B's side-s neighbours links back to B on the opposite side.
- If the margins cross, both ends collapse to the midpoint.
- f's coordinate along the line is clamped into the resulting range. z = A's height there.

### A* search (generic engine search)

Inputs: start area, optional goal area, optional goal position (one of the two must be given), a cost function, optional max path length, team.

- If the goal area is blocked for the team, it is dropped and the goal position is used instead. If start == goal area, the trivial path succeeds.
- goal_pos = the given position, else the goal area's centre. Heuristic h(area) = |center(area) − goal_pos| (Euclidean, 3D, never scaled).
- The start area gets g = cost(start, none) (0 for the stock cost) and f = h. The open list is kept sorted ascending by f. A new entry goes **after** existing entries with equal f, so ties are FIFO.
- Loop: pop the lowest f. Skip it if blocked for the team. It is the goal if it equals the goal area, or, with no goal area, if it "contains" goal_pos. Otherwise expand neighbours in this order:
  1. floor links, sides N, E, S, W, each in file order;
  2. up-ladders in list order, each giving top_forward, then top_left, then top_right (missing ones are skipped);
  3. down-ladders in list order, each giving bottom_area.
- For each neighbour n: skip it if n is the current area's parent, n is the area itself, or n is blocked. Then `g' = cost(n, current, ladder_or_none, link_length)`, where link_length is the connection length for floor links and −1 for ladders. A negative cost means impassable: skip. A NaN cost becomes 1e30. Clamp `g' = max(g', 1.00001·g(current) + 0.00001)`.
- With a max length: skip n if `path_len(current) + |center(n) − center(current)| > max_len`. Otherwise record that sum as path_len(n).
- If n is already open or closed with g(n) ≤ g', skip it. Otherwise set g(n) = g', f(n) = g' + h(n), parent = current, and record **how** (side index 0..3, or 4 = ladder up, 5 = ladder down). A closed n is reopened. An open n moves up the sorted list.
- Track the "closest area": the visited area with the smallest h seen when its cost was set. The start counts. If the search fails, the caller can still build a partial path to it.
- The current area is closed after expansion. An empty open list means no path.

### Stock path cost ("shortest path")

For entering area n from area a:
```
if a is none: return 0
dist = ladder.length          if moving by ladder
     = link_length            if link_length > 0
     = |center(n) − center(a)|  otherwise
cost = g(a) + dist
if n has CROUCH: cost += 20 · dist
if n has JUMP:   cost += 5 · dist
return cost
```

The engine uses this cost for its own travel-distance queries, such as earliest-occupy times. CS:S bots use their own cost function, which is not in the SDK (Open questions). For reference, the HL2MP bot's cost shows the style later Valve bots use:
- impassable if the edge-height rise between the two areas is ≥ max jump height, or the drop is more than the death-drop height;
- dist × 2 when the rise is ≥ step height;
- a per-bot "route preference" factor `1 + 50·(1 + cos(entindex · area_id · (floor(time/10)+1)))`, so different bots take different routes, with the choice changing every 10 s.

The edge-height rise is the z of B's portal back toward A minus the z of A's portal toward B.

### Path building (NextBot, generic)

Input: the A* result (an area chain from start to the closest or goal area, with "how" codes), the bot's start position, and the goal position.

1. Nodes: one per area in order. The first node has no "how". If the search reached the goal (or partial goals are allowed), append a final node at the goal position in the last area. The chain is capped at 256 nodes. Same-area start and goal give a 2-node path straight to the goal.
2. Node 0 position: the start position if the first area "contains" it, else that area's centre.
3. For each following node `to` with previous node `from`:
   - **Floor link** (how 0..3): store the portal (centre and half width). Position = the closest crossing point from `from.pos`, with z = **from-area** height there (the point is on the exit edge).
     - Drop test: take the ground normal of the from-area. Measure `drop = −dot(to_area_point − from_area_point, normal)`, where the points are from.pos on from-area and to.pos projected onto to-area. If drop > step height (18), push the point outward along the link direction in 10-unit steps, up to 2 × (hull width + 5), until a hull sweep down to the to-area height is clear.
     - Then look for real ground below. If the pushed point is more than step height above it, mark this node **drop-down** and insert a duplicate node right after it, at the bottom (same xy, z = ground). Ground comes from a world trace.
   - **Ladder up** (how 4): find, in from-area's up list, the ladder whose top forward, left or right area is `to.area`. Position = ladder.bottom + normal × 32 (2 × half_human_width). Type ladder-up. If no ladder is found, the path fails.
   - **Ladder down** (how 5): find, in from-area's down list, the ladder whose bottom area is `to.area`. Position = ladder.top − normal × 32. Type ladder-down.
4. Second pass, over consecutive pairs where both are floor moves on the ground. Let `cT` = the closest point on to-area to from.pos, and `cF` = the closest point on from-area to cT.
   - **Gap jump**: when the 2D distance |cF − cT| > 47.5 (1.9 × 25) and that 2D distance > 0.5 × |cT.z − cF.z|. The landing point is the closest point on to-area to to.pos. The launch point is the closest point on from-area to the landing. `fwd` = unit(landing − launch). Set to.pos = landing + fwd × hull/2. Insert a **jump-over-gap** node before it at launch − fwd × hull/2.
   - **Climb up**: otherwise, when cT.z − cF.z > step height. Set to.pos = to-area centre, and insert a **climb-up** node before it at the closest point on from-area to that centre.
5. Finish: for each node, forward = unit(next.pos − pos), length = |next.pos − pos|, distance_from_start = running sum. Curvature at interior on-ground nodes = 0.5 · (1 − dot(prev_fwd₂D, fwd₂D)), made negative if the turn is to the right. Curvature is 0 elsewhere. The last node copies the previous forward and has length 0.
6. Path smoothing exists in the code but is disabled (it returns immediately).

### Path following (NextBot, generic), once per bot update

1. If waiting for a blocker, do nothing.
2. **Ladder handling.** If the bot is mid-ladder, let locomotion finish. If the current goal is ladder-up: once the feet are above `ladder.top.z − 18`, advance the goal. Otherwise, within 50 units (2D) of `ladder.bottom`, line up with the ladder so that dot(normal₂D, direction to bottom) < −0.9. Then walk to the bottom and start climbing within 25 units. If not lined up, circle round to the front at a radius shrinking from 50 to 25. If farther than 50, follow the path normally. Ladder-down: if already below `bottom.z + 18`, skip the node (we fell). Otherwise walk to `top + normal × hull/2` and start descending within 25 units (or immediately if already attached to a ladder).
3. **Speed**: run speed. On curves it is reduced toward walk speed: `speed = run + |curvature| · (walk − run)`. Full run speed before a gap jump or while airborne.
4. **Progress**: optionally skip ahead through on-ground nodes closer than a minimum look-ahead, when the next node is directly walkable, not more than 18 above the feet, and has no gap. Then decide whether the goal is reached:
   - drop-down: when feet z − landing z < 18;
   - climb-up: when feet z > goal z + 18;
   - otherwise: either the goal has been passed, or it is within 25 units in 2D. "Passed" means the goal is behind the plane given by the sum of the previous and current segment directions, with |Δz| below the standing hull height, Δz < 18, and the next node directly walkable without a gap.
   - Reaching the last node succeeds only while on the ground.
5. Steering direction = goal − feet, flattened and then projected onto the ground plane. Climb-up nodes steer toward the node after them.
6. Climb attempts (ledge search with world traces) and gap jumps: jump when a jump-over-gap node is within 2 × hull width ahead and the ground ahead really drops away.
7. **Fell off**: not on a ladder, not jumping, not on a STAIRS area, and the goal is more than the max jump height above the feet. If the bot is stuck or within 25 units (2D), and the next node is also unreachable, report failure (the caller repaths).
8. Every 0.5 s, look for blocking actors and pause 0.5–1 s if one is found. Otherwise steer around nearby actors, except on PRECISE areas, which disable avoidance. Then face the goal (on ground) and move toward it. Approaching climb-up or gap-jump nodes forces a standing posture.

### Attribute flags

The bit values are file data. The meanings are as used by the editor and bots.

| Bit | Name (editor command, player-typable) | Meaning | Used by generic code |
|---|---|---|---|
| 0x0001 | CROUCH | must crouch to pass | stock cost ×20 penalty |
| 0x0002 | JUMP | must jump to pass | stock cost ×5 penalty |
| 0x0004 | PRECISE | follow the area exactly, no obstacle avoidance or corner cutting | follower avoidance, smoothing |
| 0x0008 | NO_JUMP | do not jump at discontinuities here | (CS bot) |
| 0x0010 | STOP | come to a stop on entering | (CS bot) |
| 0x0020 | RUN | must run | (CS bot) |
| 0x0040 | WALK | must walk | (CS bot) |
| 0x0080 | AVOID | avoid unless the alternatives are worse | (CS bot) |
| 0x0100 | TRANSIENT | may become blocked. Re-check periodically. | blocked updates |
| 0x0200 | DONT_HIDE | never generate hiding spots here | analysis |
| 0x0400 | STAND | bots hiding here should stand | (CS bot) |
| 0x0800 | NO_HOSTAGES | hostages must not use this area | (CS hostages) |
| 0x1000 | STAIRS | stairs: walk up, don't jump | follower "fell off" check |
| 0x2000 | NO_MERGE | editor only | - |
| 0x4000 | OBSTACLE_TOP | top of a climbable obstacle | - |
| 0x8000 | CLIFF | next to a drop of at least 300 units | - |
| 0x10000..0x04000000 | game custom | | |
| 0x20000000, 0x40000000, 0x80000000 | runtime only (designer cost, elevator, blocker) | should never be set in a file. Mask them off on load. | |

In a version-9 file only the low 16 bits exist (u16).

"Blocked" is a runtime state per team (doors, func_nav_blocker and similar). It is not stored. A* and the area queries skip blocked areas.

## Per-tick order

There is no fixed tick in the nav system. Per bot update (the generic follower):
1. ladder handling (it may consume the update);
2. speed from curvature;
3. progress check and goal advance (it may finish the path);
4. steering vectors;
5. climb or gap-jump attempts;
6. fell-off check (it may fail the path);
7. blocker wait / actor avoidance (0.5 s interval);
8. face and approach.

Path (re)computation is driven by the bot's behaviour, not by the follower.

## Edge cases

- Shared edges overlap both areas (inclusive tests). "Area at position" then picks the higher one, and on equal heights the one found first. Tests should not use points exactly on an edge.
- Degenerate areas (zero width or depth) report ne_z as their height everywhere. None exist in stock CS:S files.
- An area under a higher one: "area at position" picks the highest area at or below p.z + 5 that is within 120 below. A point floating more than 120 above the floor returns none (so call nearest-area).
- A dangling connection id (not in the mesh) marks the mesh corrupt but does not stop loading. None in stock dust2.
- Ladders with bottom_area = 0, or no top areas, are broken. Skip them for pathing.
- Hiding spot id 0 is a real spot (dust2 spot ids start at 0). Encounter records also write 0 for "no spot". This is ambiguous only in edited, unanalysed meshes.
- Strings in the place list may be longer than 255 bytes in theory (u16 length). The engine truncates at 256 bytes. We should simply honour the length.
- The bsp_size check is advisory only. Ignore it, or warn.

## Quirks

- **Centre z is not the surface height at the centre.** It is the mean of the NW and SE heights. A* distances, the heuristic and connection lengths all use this centre. Keep it this way so path costs match the original.
- **Two format versions ship.** The 3 newest-shipped maps (dust, dust2, inferno) are v16 with sub-version 1, and have *no* encounter spots, *no* approach data, and **default earliest-occupy times (120, 120) on every area**, although the header says "analysed". Their light values are all 1.0. The 15 v9 maps carry real approach data, encounter spots and occupy times. If CS:S bots rely on encounter spots for "check corners" behaviour, they lose it on dust/dust2/inferno. That is a fact about the shipped data. Keep it.
- **Ladder length is stored, not derived**, and can differ from the endpoint distance. Use the stored value as cost.
- **A* never uses a ladder's "behind" top area going up**, though descending can start from it if the area lists the ladder as "down".
- At load, the SDK's stair re-test clears the STAIRS bit. When it decides an area *is* stairs, it sets the flags to exactly STAIRS, **wiping every other bit** on that area. Whether the CS:S binary does the same is unknown (Open questions). Our loader keeps the stored flags.
- Approach records (v9) are read and thrown away by the shared loader. Only the CS bot used them.

## Test cases

All numbers come from the stock files in the user's CS:S install (`cstrike_pak_dir.vpk` → `maps/<map>.nav`). The A* row was computed with the stock cost exactly as described above, in 64-bit floats, so compare with a tolerance of ±0.05.

### Header and totals

| Setup | Input | After | Expected |
|---|---|---|---|
| de_dust2.nav | parse | header | magic 0xFEEDFACE, version 16, subversion 1, bsp_size 16487028, analyzed 1 |
| de_dust2.nav | parse | place list | 9 names, in order: Side, CTSpawn, TSpawn, BombsiteA, Middle, BombsiteB, Tunnel, SideDoor, DoubleDoors; has_unnamed_areas = 1 |
| de_dust2.nav | parse | counts | 961 areas, 0 ladders, 536 hiding spots (ids 0..535), 0 encounters, 3114 floor links (64 one-way), 59709 visible-area entries, 558 areas with non-zero inherit-visibility |
| de_dust2.nav | parse | end | the reader ends at exactly byte 420178 (file size). The per-area CS byte is 0 on all 961 areas. The ladder count (last 4 bytes) is 0. |
| de_dust2.nav | parse | per-place area counts | none 3, Side 107, CTSpawn 60, TSpawn 201, BombsiteA 120, Middle 95, BombsiteB 200, Tunnel 104, SideDoor 41, DoubleDoors 30 |
| de_dust2.nav | parse | flag bit counts | 0x2: 70, 0x4: 5, 0x8: 84, 0x40: 2, 0x200: 28, 0x400: 8, 0x1000: 11; no other bits set |
| de_dust2.nav | parse | hiding spot flags | 290 spots with flags 8 (exposed), 246 with flags 1 (in cover) |
| de_dust2.nav | parse | occupy / light | every area: earliest_occupy (120, 120), light (1, 1, 1, 1) |
| de_dust2.nav | parse | extent | min nw.x −2200, max se.x 1781.1792, min nw.y −1000, max se.y 3125 → grid 14 × 14 |
| de_dust.nav | parse | | v16 sub 1, 1059 areas, 10 places, has_unnamed 0, 557 hiding spots, 0 ladders |
| de_inferno.nav | parse | | v16 sub 1, 1664 areas, 19 places, 770 hiding spots, 0 ladders |
| de_train.nav | parse | | **v9** (no subversion, no analyzed byte, no unnamed byte, u16 flags, no light/visibility, no CS byte), bsp_size 11091880, 12 places (first: CTSpawn, BackAlley, LongHall), 1335 areas, 41 ladders, 512 hiding spots, 14838 approach records, 14226 encounters, 56 area→ladder links; ends exactly at file end (628133 bytes) |
| cs_office.nav | parse | | v9, 1002 areas, 2 ladders, 14 places (first CTSpawn, Garage, TSpawn) |
| all 18 stock .nav | parse | end | every file decodes to exactly its size. v16: de_dust, de_dust2, de_inferno. v9: the other 15. |

### Specific dust2 areas

| Setup | Input | After | Expected |
|---|---|---|---|
| dust2 area index 0 | parse | record (starts at byte 117) | id 2, flags 0, nw (1075, 800, 1.8456212), se (1450, 1175, 0.27639693), ne_z 3.8144591, sw_z 0.93932509; N → [3669, 2771, 2787, 3682], E → [147, 592, 2568], S → [1092, 2773, 2774, 3982, 3983], W → [124, 23, 2532]; 0 hiding spots; place 1 (Side); 45 visible entries; inherit-visibility 2787 |
| dust2 area index 1 | parse | record (byte 506) | id 3, flags 0, nw (100, 2050, −125.77076), se (450, 2400, −120.77489), ne_z −120.48857, sw_z −124.43034; link counts N 0, E 4 [131, 2555, 4148, 4150], S 3 [241, 243, 223], W 3 [53, 775, 3526]; place 2 (CTSpawn); 168 visible entries; inherit 0 |
| dust2 area id 10 | parse | | flags 0, nw (−550, 1950, −126.78474), se (−300, 2300, −125.94894); links N [51, 326], E [1515, 4119, 4121, 4125], S [1464, 1431, 481, 325], W [169, 2673, 2674]; 1 hiding spot: id 0, pos (−312.5, 1962.5, −126.30378), flags 8; first visible entries (3, vis 1), (3726, 3), (3729, 1) |
| dust2 last area | parse | | id 4181, flags 0x400 (STAND), nw (−475, −675, 123.36160), se (−450, −650, 117.89486); one link N → 4172; hiding spot id 535 at (−462.5, −662.5, 117.89486), flags 8; place 3 (TSpawn) |
| dust2 areas 8 and 36 | parse | | both flat at z 96.031258. Area 8: nw (275, 2225), se (525, 2500). Area 36: nw (275, 2050), se (450, 2225). Both place 4 (BombsiteA). |
| dust2 one-way link | parse | | area 8 links East to 2096, but 2096 has no West link back to 8 |

### Geometry and queries (dust2)

| Setup | Input | After | Expected |
|---|---|---|---|
| area 2 | height at its centre (1262.5, 987.5) | | 1.7189506 (stored centre z is 1.0610091: the NW/SE mean) |
| area 2 | height at (1450, 800) (NE corner) | | 3.8144591 (= ne_z) |
| area 2 | height at (0, 0) (outside → clamped to NW) | | 1.8456212 |
| mesh | area at position (275, 2225, −100) | | id 3, surface z ≈ −122.866 |
| mesh | area at position (400, 2300, 150) | | id 8 (z 96.03; area 3 below is > 120 down) |
| mesh | area at position (275, 2225, 7) | | none (area 3 is 130 below, > 120; areas 8/36 at z 96 are above) |
| mesh | area at position (1200, 1000, 40) | | id 2, z ≈ 1.5507 |
| mesh | area at position (1200, 1000, 200) | | none |
| mesh | portal from area 2 across its West side to area 124 | | x = 1075; y range = overlap of [800, 1175] with area 124's y range (clamped to [800, 1175]) |

### Pathfinding (dust2, stock cost)

| Setup | Input | After | Expected |
|---|---|---|---|
| A* start area 3, goal area 5 (goal pos = area 5 centre (−1775, 275, 3.4470096)) | search | path | 31 areas: 3, 53, 4139, 4121, 10, 51, 240, 493, 785, 181, 77, 2691, 28, 137, 3948, 222, 2819, 1706, 1705, 1700, 1697, 2861, 2687, 39, 3930, 3860, 3864, 3861, 3218, 252, 5; final g = 3676.23 |
| A* start area 5, goal area 3 | search | cost | 31 areas, g = 3676.23 |

### Ladders (de_train, v9)

| Setup | Input | After | Expected |
|---|---|---|---|
| de_train ladder record 0 | parse | | id 1, width 24, top (424, 720, 40.031254), bottom (424, 720, −200), length 240, dir 1 (East), top_forward 67, top_left 512, top_right 0, top_behind 0, bottom 948 |
| de_train ladder record 1 | parse | | id 2, width 24, top (285, 546, −78.13804), bottom (285, 546, −160), length 94 (≠ endpoint distance 81.86), dir 2 (South), top_forward 190, others 0, bottom 658 |
| de_train area index 0 | parse | | id 1, flags 0, nw (1525, −1250, −323.96875), se (1775, −975, −327.96875), link counts N 2, E 2, S 4, W 4; 0 hiding spots; 3 approach records; 132 encounters; place 1 (CTSpawn); occupy (120, 120) |
| de_train, all areas with place CTSpawn | mean earliest_occupy | | [0] ≈ 28.68, [1] ≈ 9.65 (confirms that index 1 is the CT team) |
| de_train area index 1 | parse | | occupy (4.1020765, 16.093021); first approach records (919, 922, 1, 920, 1), (678, 330, 1, 679, 1); first encounter from 195 dir 0 → to 130 dir 0 with 0 spots |

## Open questions

1. **What is the per-area CS byte in v16/sub-version 1 files?** All three files store a single 0 there for every area, so the entry size cannot be checked. Best guess: the CS area subclass's approach-area list, in the same 14-byte layout as v<15 approach records. Check by saving a mesh after `nav_analyze` in CS:S (`sv_cheats 1; nav_edit 1; nav_analyze`) and inspecting the bytes. Without that, a parser that meets a non-zero count cannot safely continue.
2. **CS:S bot path cost.** Not in the SDK. We don't know whether it uses the CROUCH/JUMP penalties above, penalises AVOID areas, uses danger values, rejects drops over 200 (death_drop), or adds per-bot route randomness. Check: watch CS:S bots on a map with alternatives (`bot_stop 1`, `nav_show_*` and similar), or write a dedicated behaviour spec from whatever CS-bot material is legitimately available.
3. **How CS:S bots honour RUN/WALK/STOP/NO_JUMP/STAND/CROUCH/PRECISE while following.** The generic NextBot follower ignores most of these. The CS bot follower is not in the SDK.
4. **Does CS:S use the NextBot path follower at all?** Probably not. The CS bot predates NextBot and has its own path builder and follower, so the follower section above is a reference design, not CS:S behaviour.
5. **Nearest-area distance**: the SDK measures from the original position. A source note says this change "needs porting back to CS:S", which suggests CS:S measures from the raised source point (ground + 35.5) instead. Test: place a bot off-mesh above a ledge and see which area it picks.
6. **Stairs re-test at load**: does the CS:S binary wipe the other flags on areas it marks as stairs, as the SDK does? Compare `nav_edit` attribute display on a stairs area in dust2 (11 stored STAIRS areas) with the file.
7. **Approach record field meaning (v9)**: field order and semantics are inferred. Only the 14-byte size is certain from source.
8. Whether `earliest_occupy` (all 120 in v16 maps) changes CS:S bot round-start behaviour on dust2 compared with v9 maps.
