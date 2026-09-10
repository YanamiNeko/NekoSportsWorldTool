//! GT4 缺口识别图像管线。
//!
//! 灰度 → 高斯模糊(5x5, σ=1.1) → Canny(100,200) → 零均值归一化互相关模板匹配，
//! 返回最佳匹配 x。

/// PNG 解码 → 灰度（RGBA 合成到黑底）。
pub fn decode_gray(png: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let img = image::load_from_memory(png).map_err(|e| format!("PNG 解码失败: {e}"))?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    let mut gray = vec![0u8; (w * h) as usize];
    for (i, px) in rgba.pixels().enumerate() {
        let [r, g, b, a] = px.0;
        // alpha 合成到黑底
        let (r, g, b) = (
            r as u32 * a as u32 / 255,
            g as u32 * a as u32 / 255,
            b as u32 * a as u32 / 255,
        );
        gray[i] = ((r * 299 + g * 587 + b * 114) / 1000) as u8;
    }
    Ok((gray, w as usize, h as usize))
}

/// 可分离高斯模糊（5x5，σ≈1.1）。
pub fn gaussian_blur(src: &[u8], w: usize, h: usize) -> Vec<f32> {
    // σ = 0.3*((k-1)*0.5-1)+0.8, k=5
    let sigma = 1.1f32;
    let mut kernel = [0f32; 5];
    for (i, k) in kernel.iter_mut().enumerate() {
        let x = i as f32 - 2.0;
        *k = (-x * x / (2.0 * sigma * sigma)).exp();
    }
    let sum: f32 = kernel.iter().sum();
    for k in kernel.iter_mut() {
        *k /= sum;
    }
    let mut tmp = vec![0f32; w * h];
    let mut dst = vec![0f32; w * h];
    // 水平
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0f32;
            for (ki, k) in kernel.iter().enumerate() {
                let xx = (x as i32 + ki as i32 - 2).clamp(0, w as i32 - 1) as usize;
                acc += src[y * w + xx] as f32 * k;
            }
            tmp[y * w + x] = acc;
        }
    }
    // 垂直
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0f32;
            for (ki, k) in kernel.iter().enumerate() {
                let yy = (y as i32 + ki as i32 - 2).clamp(0, h as i32 - 1) as usize;
                acc += tmp[yy * w + x] * k;
            }
            dst[y * w + x] = acc;
        }
    }
    dst
}

/// Canny 边缘检测（低/高阈值 + 滞后连接，输出 0/255）。
pub fn canny(src: &[f32], w: usize, h: usize, low: f32, high: f32) -> Vec<u8> {
    // Sobel 梯度
    let mut mag = vec![0f32; w * h];
    let mut dir = vec![0f32; w * h];
    let at = |x: i32, y: i32| -> f32 {
        src[(y.clamp(0, h as i32 - 1) as usize) * w + (x.clamp(0, w as i32 - 1) as usize)]
    };
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            let gx = -at(x - 1, y - 1) - 2.0 * at(x - 1, y) - at(x - 1, y + 1)
                + at(x + 1, y - 1) + 2.0 * at(x + 1, y) + at(x + 1, y + 1);
            let gy = -at(x - 1, y - 1) - 2.0 * at(x, y - 1) - at(x + 1, y - 1)
                + at(x - 1, y + 1) + 2.0 * at(x, y + 1) + at(x + 1, y + 1);
            let i = (y * w as i32 + x) as usize;
            mag[i] = (gx * gx + gy * gy).sqrt();
            dir[i] = gy.atan2(gx);
        }
    }
    // 非最大值抑制
    let mut nms = vec![0f32; w * h];
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let i = y * w + x;
            let mut a = dir[i];
            if a < 0.0 {
                a += std::f32::consts::PI;
            }
            let (dx, dy) = if a < std::f32::consts::FRAC_PI_4 {
                (1, 0)
            } else if a < std::f32::consts::FRAC_PI_2 {
                (1, 1)
            } else if a < 3.0 * std::f32::consts::FRAC_PI_4 {
                (0, 1)
            } else {
                (-1, 1)
            };
            let m = mag[i];
            let m1 = mag[((y as i32 + dy) as usize) * w + ((x as i32 + dx) as usize)];
            let m2 = mag[((y as i32 - dy) as usize) * w + ((x as i32 - dx) as usize)];
            if m >= m1 && m >= m2 {
                nms[i] = m;
            }
        }
    }
    // 双阈值 + 滞后连接（BFS）
    let mut out = vec![0u8; w * h];
    let mut stack = Vec::new();
    for i in 0..w * h {
        if nms[i] >= high {
            out[i] = 255;
            stack.push(i);
        }
    }
    while let Some(i) = stack.pop() {
        let x = (i % w) as i32;
        let y = (i / w) as i32;
        for dy in -1..=1 {
            for dx in -1..=1 {
                let nx = x + dx;
                let ny = y + dy;
                if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                    continue;
                }
                let j = (ny * w as i32 + nx) as usize;
                if out[j] == 0 && nms[j] >= low {
                    out[j] = 255;
                    stack.push(j);
                }
            }
        }
    }
    out
}

/// 零均值归一化互相关模板匹配（TM_CCOEFF_NORMED 等价），返回最佳 x。
pub fn match_x(bg: &[u8], bw: usize, bh: usize, tpl: &[u8], tw: usize, th: usize) -> usize {
    if tw == 0 || th == 0 || tw > bw || th > bh {
        return 0;
    }
    // 模板零均值
    let t_mean = tpl.iter().map(|&v| v as f64).sum::<f64>() / (tw * th) as f64;
    let t: Vec<f64> = tpl.iter().map(|&v| v as f64 - t_mean).collect();
    let t_norm: f64 = t.iter().map(|v| v * v).sum::<f64>().sqrt();
    if t_norm == 0.0 {
        return 0;
    }
    let mut best_x = 0usize;
    let mut best_score = f64::NEG_INFINITY;
    for oy in 0..=(bh - th) {
        for ox in 0..=(bw - tw) {
            let mut s_sum = 0f64;
            let mut st_sum = 0f64;
            for ty in 0..th {
                let row = (oy + ty) * bw + ox;
                let trow = ty * tw;
                for tx in 0..tw {
                    let v = bg[row + tx] as f64;
                    s_sum += v;
                }
                let _ = trow;
            }
            let s_mean = s_sum / (tw * th) as f64;
            let mut num = 0f64;
            for ty in 0..th {
                let row = (oy + ty) * bw + ox;
                let trow = ty * tw;
                for tx in 0..tw {
                    let sv = bg[row + tx] as f64 - s_mean;
                    num += sv * t[trow + tx];
                }
            }
            // 窗口能量（复用 t 归一化的分母需按窗口重算——精确起见直接算）
            for ty in 0..th {
                let row = (oy + ty) * bw + ox;
                for tx in 0..tw {
                    let sv = bg[row + tx] as f64 - s_mean;
                    st_sum += sv * sv;
                }
            }
            let denom = st_sum.sqrt() * t_norm;
            let score = if denom == 0.0 { 0.0 } else { num / denom };
            if score > best_score {
                best_score = score;
                best_x = ox;
            }
        }
    }
    best_x
}

/// getDistance_original：bg/slice 两张 PNG → 缺口 x。
pub fn get_distance_original(bg_png: &[u8], slice_png: &[u8]) -> Result<usize, String> {
    let (bg_gray, bw, bh) = decode_gray(bg_png)?;
    let (slice_gray, sw, sh) = decode_gray(slice_png)?;
    let bg_blur = gaussian_blur(&bg_gray, bw, bh);
    let bg_edges = canny(&bg_blur, bw, bh, 100.0, 200.0);
    let slice_blur = gaussian_blur(&slice_gray, sw, sh);
    let slice_edges = canny(&slice_blur, sw, sh, 100.0, 200.0);
    if slice_edges.iter().filter(|&&v| v == 255).count() < 10 {
        return Err("切片图无边缘特征（识别失败）".into());
    }
    Ok(match_x(&bg_edges, bw, bh, &slice_edges, sw, sh))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成图像自测：平滑背景 + 明显亮块 → canny 找得到边缘，模板匹配定位准确。
    #[test]
    fn test_match_x_synthetic() {
        // 100x60 平滑背景（无 u8 环绕）
        let (w, h) = (100usize, 60usize);
        let mut bg = vec![0u8; w * h];
        for y in 0..h {
            for x in 0..w {
                bg[y * w + x] = ((x + y) / 2) as u8;
            }
        }
        // 在 x=37 处放一个 20x20 的亮块（模拟缺口）
        for y in 10..30 {
            for x in 37..57 {
                bg[y * w + x] = 255;
            }
        }
        let bg_blur = gaussian_blur(&bg, w, h);
        let bg_edges = canny(&bg_blur, w, h, 50.0, 120.0);
        assert!(bg_edges.iter().filter(|&&v| v == 255).count() > 20, "边缘太少");
        // 模板 = x=30 起裁 30x30（亮块 + 周边背景，真实切片同构）
        let (tw2, th2) = (30usize, 30usize);
        let mut tpl = vec![0u8; tw2 * th2];
        for y in 0..th2 {
            for x in 0..tw2 {
                tpl[y * tw2 + x] = bg[(5 + y) * w + 30 + x];
            }
        }
        let t_edges = canny(&gaussian_blur(&tpl, tw2, th2), tw2, th2, 50.0, 120.0);
        let found = match_x(&bg_edges, w, h, &t_edges, tw2, th2);
        assert!((found as i32 - 30).abs() <= 2, "匹配 x={found} 期望≈30");
    }
}
