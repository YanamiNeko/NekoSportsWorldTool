//! API 层：HTTP 客户端 + 各业务接口 + GT4 验证 + 全链编排。

pub mod ai;
pub mod campus;
pub mod map_boundary;
pub(crate) mod map_search;
pub mod cheat;
pub mod client;
pub mod fence;
pub mod flow;
pub mod gt4;
pub mod gt4image;
pub mod login;
pub mod model;
pub mod obs;
pub mod points;
pub mod policy;
pub mod rank;
pub mod records;
pub mod school_roads;
pub mod semester;
pub mod submit;
pub(crate) mod submission_guard;
pub mod user;
