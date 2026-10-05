//! 深圳市房地产信息平台（fdc.zjj.sz.gov.cn）楼盘交易状况查询。
//!
//! 纯 HTTP 直连官方 API（v0.2.0 方案）：瑞数（RiverSecurity）反爬只挂在 HTML
//! 首页上，`/szfdcscjy/*` 接口路径实测**无需 cookie 直接 200**，带常规 Chrome UA
//! + Origin + Referer 头 POST 即可取数。
//!
//! **零外部软件依赖**：不启动浏览器、不依赖本机 Chrome/Edge/Playwright 缓存等
//! 任何本地软件，Windows / Ubuntu 完全一致，静态编译单二进制即可运行。

use anyhow::{bail, Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

/// 常规 Chrome UA（API 层同样需要，模拟真实浏览器访问）
const USER_AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";
const BASE: &str = "https://fdc.zjj.sz.gov.cn";

fn parse_resp(s: &str, path: &str) -> Result<Value> {
    let resp: Value = serde_json::from_str(s)
        .map_err(|e| anyhow::anyhow!("响应不是 JSON: {e}: {}", &s[..s.len().min(200)]))?;
    let status = resp["status"].as_i64().unwrap_or(0);
    if status != 200 {
        bail!("接口 {} 返回 {}: {}", path, status, resp["msg"].as_str().unwrap_or("?"));
    }
    Ok(resp["data"].clone())
}

/// POST JSON
fn post_json(client: &reqwest::blocking::Client, path: &str, body: &Value) -> Result<Value> {
    let resp = client
        .post(BASE.to_owned() + path)
        .header("Content-Type", "application/json")
        .header("Origin", BASE)
        .header("Referer", format!("{BASE}/szfdcscjy/index.html"))
        .body(body.to_string())
        .send()
        .with_context(|| format!("请求 {path} 失败"))?;
    parse_resp(&resp.text().context("读取响应失败")?, path)
}

/// POST form-urlencoded
fn post_form(client: &reqwest::blocking::Client, path: &str, body: &str) -> Result<Value> {
    let resp = client
        .post(BASE.to_owned() + path)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .header("Origin", BASE)
        .header("Referer", format!("{BASE}/szfdcscjy/index.html"))
        .body(body.to_string())
        .send()
        .with_context(|| format!("请求 {path} 失败"))?;
    parse_resp(&resp.text().context("读取响应失败")?, path)
}

struct Project {
    id: String,
    syp_id: String,
    name: String,
    license: String,
    zone: String,
    developer: String,
    address: String,
    passdate: String,
}

/// 按楼盘名称搜索一手预售项目
fn search_projects(client: &reqwest::blocking::Client, name: &str) -> Result<Vec<Project>> {
    let body = serde_json::json!({
        "project": name,
        "pageIndex": 1,
        "pageSize": 50,
        "total": 0,
        "zone": ""
    });
    let data = post_json(client, "/szfdcscjy/ysf/publicity/getYsfYsPublicity", &body)?;
    let list = data["list"].as_array().cloned().unwrap_or_default();
    let mut out = Vec::new();
    for it in &list {
        out.push(Project {
            id: it["id"].as_str().unwrap_or_default().to_string(),
            syp_id: it["sypId"].as_str().unwrap_or_default().to_string(),
            name: it["project"].as_str().unwrap_or_default().to_string(),
            license: it["strpreprojectid"].as_str().unwrap_or_default().to_string(),
            zone: it["zone"].as_str().unwrap_or_default().to_string(),
            developer: it["name"].as_str().unwrap_or_default().to_string(),
            address: it["siteaddress"].as_str().unwrap_or_default().to_string(),
            passdate: it["passdate"].as_str().unwrap_or_default().to_string(),
        });
    }
    Ok(out)
}

/// 项目详情（含楼栋列表）。返回 (项目信息, 楼栋 id/名称 列表)
fn project_detail(
    client: &reqwest::blocking::Client,
    presell_id: &str,
) -> Result<(Value, Vec<(String, String)>)> {
    let data = post_form(
        client,
        "/szfdcscjy/projectPublish/getProjectByPreSellId",
        &format!("preSellId={presell_id}"),
    )?;
    let buildings = data["iszYsProjectBuildingVoList"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .map(|b| {
            (
                b["id"].as_str().unwrap_or_default().to_string(),
                b["name"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let info = data["iszYsProjectBaseInfoVo"].clone();
    Ok((info, buildings))
}

/// 房源销售状态统计（按套数降序）+ 备案均价（非空备案单价均值）
fn house_status(
    client: &reqwest::blocking::Client,
    presell_id: &str,
    fyb_id: &str,
    syp_id: &str,
) -> Result<(Vec<(String, usize)>, Option<f64>)> {
    let body = serde_json::json!({
        "status": -1,
        "floor": "",
        "buildingbranch": "",
        "fybId": fyb_id,
        "preSellId": presell_id,
        "ysProjectId": syp_id,
        "type": "",
        "useage": ""
    });
    let data = post_json(client, "/szfdcscjy/projectPublish/getHouseInfoListToPublicity", &body)?;
    let mut stat: Vec<(String, usize)> = Vec::new();
    let mut prices: Vec<f64> = Vec::new();
    for floor in data.as_array().cloned().unwrap_or_default() {
        for h in floor["list"].as_array().cloned().unwrap_or_default() {
            let status = h["lastStatusName"].as_str().unwrap_or_default().to_string();
            match stat.iter_mut().find(|(s, _)| *s == status) {
                Some((_, c)) => *c += 1,
                None => stat.push((status, 1)),
            }
            if let Some(p) = h["recordedPricePerUnitInside"].as_f64() {
                prices.push(p);
            }
        }
    }
    stat.sort_by(|a, b| b.1.cmp(&a.1));
    let avg_price = if prices.is_empty() {
        None
    } else {
        Some(prices.iter().sum::<f64>() / prices.len() as f64)
    };
    Ok((stat, avg_price))
}

/// szhousing 任务入口：纯 HTTP 直查平台官方接口，返回文本报表。
///
/// `args["name"]`（或 `input`）为楼盘名称（模糊匹配）；未提供时返回近期在售
/// 预售项目列表（第一页）。
pub fn report(args: &HashMap<String, String>) -> Result<String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent(USER_AGENT)
        .build()
        .context("创建 HTTP 客户端失败")?;
    let name = args
        .get("name")
        .or_else(|| args.get("input"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    match name {
        Some(name) => project_report(&client, &name),
        None => recent_projects_report(&client),
    }
}

/// 指定楼盘：项目信息 + 各楼栋销售状态统计
fn project_report(client: &reqwest::blocking::Client, name: &str) -> Result<String> {
    let projects = search_projects(client, name)?;
    if projects.is_empty() {
        bail!("未找到名称包含「{name}」的在售预售项目");
    }
    let pick = projects
        .iter()
        .find(|p| p.name == name)
        .unwrap_or(&projects[0]);

    let (_info, buildings) = project_detail(client, &pick.id)?;
    let mut out = String::new();
    out.push_str(&format!("楼盘：{}\n", pick.name));
    out.push_str(&format!("预售证：{}\n", pick.license));
    out.push_str(&format!("区域：{}\n", pick.zone));
    out.push_str(&format!("开发商：{}\n", pick.developer));
    out.push_str(&format!("地址：{}\n", pick.address));
    out.push_str(&format!("批准时间：{}\n", pick.passdate));

    if projects.len() > 1 {
        out.push_str(&format!("\n共匹配 {} 个项目（显示第一个），其余：\n", projects.len()));
        for p in projects.iter().filter(|p| p.name != pick.name).take(5) {
            out.push_str(&format!("  - {} [{}] {} 批准时间 {}\n", p.name, p.zone, p.license, p.passdate));
        }
    }

    for (bid, bname) in &buildings {
        let (stats, avg_price) = house_status(client, &pick.id, bid, &pick.syp_id)?;
        let total: usize = stats.iter().map(|(_, c)| *c).sum();
        out.push_str(&format!("\n楼栋 {}：总套数 {}\n", bname, total));
        for (s, c) in &stats {
            let pct = if total > 0 {
                format!("{:.1}%", *c as f64 / total as f64 * 100.0)
            } else {
                "0%".to_string()
            };
            out.push_str(&format!("  {s}：{c} 套（{pct}）\n"));
        }
        if let Some(avg) = avg_price {
            out.push_str(&format!("  备案均价：{:.2} 元/㎡\n", avg));
        }
    }
    out.push_str("\n数据来源：深圳市房地产信息平台 https://fdc.zjj.sz.gov.cn/");
    Ok(out)
}

/// 未指定楼盘：返回近期在售预售项目列表（第一页）
fn recent_projects_report(client: &reqwest::blocking::Client) -> Result<String> {
    let projects = search_projects(client, "")
        .with_context(|| "未提供楼盘名称且获取在售项目列表失败；可传 name 参数指定楼盘（如 name:星悦尊府）")?;
    if projects.is_empty() {
        bail!("未获取到在售预售项目列表；可传 name 参数指定楼盘（如 name:星悦尊府）");
    }
    let mut out = format!("近期在售预售项目（前 {} 个）：\n", projects.len());
    for p in &projects {
        out.push_str(&format!(
            "  - {} [{}] 预售证 {} 开发商 {} 批准时间 {}\n",
            p.name, p.zone, p.license, p.developer, p.passdate
        ));
    }
    out.push_str("\n查询具体楼盘销售统计请提供 name 参数（如 name:星悦尊府）。");
    out.push_str("\n数据来源：深圳市房地产信息平台 https://fdc.zjj.sz.gov.cn/");
    Ok(out)
}
