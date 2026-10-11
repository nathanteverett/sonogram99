// The same light representation as core::glow: (energy, energy * gradient position).
// Explicit samples retain the software renderer's border and palette behavior.
@group(0) @binding(0) var<storage, read> source: array<u32>;
@group(0) @binding(1) var<storage, read> auxiliary: array<u32>;
@group(0) @binding(2) var<storage, read> extra: array<u32>;
@group(0) @binding(3) var<storage, read_write> output: array<u32>;
@group(0) @binding(4) var<uniform> params: array<vec4<f32>, 16>;

fn f(i: u32) -> f32 { return params[i / 4u][i % 4u]; }
fn width() -> u32 { return u32(f(0)); }
fn height() -> u32 { return u32(f(1)); }
fn light(i: u32) -> vec2<f32> { return bitcast<vec2<f32>>(vec2(source[2u*i], source[2u*i+1u])); }
fn put(i: u32, v: vec2<f32>) { output[2u*i] = bitcast<u32>(v.x); output[2u*i+1u] = bitcast<u32>(v.y); }
fn rgb(p: u32) -> vec3<f32> { return vec3<f32>(f32((p >> 16u) & 255u), f32((p >> 8u) & 255u), f32(p & 255u)); }
fn packed(c: vec3<f32>) -> u32 { let p = vec3<u32>(clamp(c, vec3(0.0), vec3(255.0))); return (p.x << 16u) | (p.y << 8u) | p.z; }

// Projection and color resolution share one dispatch and one readback.
fn projection_color(value: vec2<f32>) -> u32 {
    let bg = params[10].xyz; let hot = params[11].xyz; let e = value.x;
    if e < 0.001 { return packed(bg); }
    let col = rgb(extra[u32(clamp(value.y/e,0.0,1.0)*255.0)]);
    let lit = 1.0-1.0/(1.0+e*(1.0+e*(0.5+e/6.0)));
    let heat = clamp((e-3.0)*0.06,0.0,0.16);
    return packed(bg+(col+(hot-col)*heat-bg)*lit);
}

fn sharp(s: vec2<f32>, keep: f32, w: u32, h: u32) -> vec2<f32> {
    if !(all(s >= vec2(0.0)) && all(s < vec2<f32>(f32(w)-1.0, f32(h)-1.0))) { return vec2(0.0); }
    let at = vec2<u32>(s); let t = fract(s); let i = at.y*w+at.x;
    return light(i)*((1.0-t.x)*(1.0-t.y)*keep) + light(i+1u)*(t.x*(1.0-t.y)*keep)
        + light(i+w)*((1.0-t.x)*t.y*keep) + light(i+w+1u)*(t.x*t.y*keep);
}

fn sampled(s: vec2<f32>, keep: f32) -> vec2<f32> {
    let w = width(); let h = height();
    if f(2) == 0.0 { return sharp(s, keep, w, h); }
    if !(all(s >= vec2(1.0)) && all(s < vec2<f32>(f32(w)-2.0, f32(h)-2.0))) { return vec2(0.0); }
    let at = vec2<u32>(s); let t = fract(s);
    let kx = vec4(1.0-t.x, 2.0-t.x, 1.0+t.x, t.x);
    let ky = vec4(1.0-t.y, 2.0-t.y, 1.0+t.y, t.y) * (keep/16.0);
    var v = vec2(0.0);
    for (var y = 0u; y < 4u; y++) {
        var row = vec2(0.0);
        for (var x = 0u; x < 4u; x++) { row += light((at.y+y-1u)*w+at.x+x-1u)*kx[x]; }
        v += row*ky[y];
    }
    return v;
}

@compute @workgroup_size(8, 8)
fn warp_flow(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let dx = f32(id.x)-f(6); let dy = f32(id.y)-f(7); let m = f(15);
    let u = dy/m; let v = dx/m;
    let fx = (sin(u*8.0+f(12)*0.7)+0.4*sin(u*15.0-f(12)*0.4))*m*f(13)*f(14);
    let fy = (sin(v*7.0-f(12)*0.6)+0.4*sin(v*13.0+f(12)*0.5))*m*f(13)*f(14);
    let sx = f(4)+f(8)*dy*f(10)+f(9)*dx*f(10)+fx;
    let sy = f(5)+f(9)*dy*f(10)-f(8)*dx*f(10)+fy;
    put(id.y*width()+id.x, sampled(vec2(sx, sy), f(11)));
}

fn corner(i: u32) -> vec3<f32> {
    return bitcast<vec3<f32>>(vec3(auxiliary[3u*i], auxiliary[3u*i+1u], auxiliary[3u*i+2u]));
}
@compute @workgroup_size(8, 8)
fn warp_grid(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let q = vec2<f32>(id.xy)*vec2(f(4)/(f(0)-1.0), f(5)/(f(1)-1.0));
    let cell = min(vec2<u32>(q), vec2<u32>(u32(f(4))-1u, u32(f(5))-1u));
    let t = q-vec2<f32>(cell); let i = cell.y*(u32(f(4))+1u)+cell.x;
    let a = mix(corner(i), corner(i+u32(f(4))+1u), t.y);
    let b = mix(corner(i+1u), corner(i+u32(f(4))+2u), t.y);
    let s = mix(a,b,t.x);
    put(id.y*width()+id.x, sampled(s.xy, s.z));
}

@compute @workgroup_size(8, 8)
fn resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let i = id.y*width()+id.x; let value = light(i); let e = value.x;
    let bg = params[1].xyz; let hot = params[2].xyz;
    if e < 0.001 { output[i] = packed(bg); return; }
    let col = rgb(auxiliary[u32(clamp(value.y/e, 0.0, 1.0)*255.0)]);
    let lit = (1.0-1.0/(1.0+e*(1.0+e*(0.5+e/6.0))))*f(2);
    let heat = clamp((e-3.0)*0.06, 0.0, 0.16);
    output[i] = packed(bg + (col+(hot-col)*heat-bg)*lit);
}

@compute @workgroup_size(8, 8)
fn post(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let i = id.y*width()+id.x; var color = rgb(source[i]);
    if f(2) > 0.001 {
        let mid = vec2(f(0), f(1))*0.5-vec2(0.5);
        let s = clamp(mid+(vec2<f32>(id.xy)-mid)*vec2(f(4), f(5))/f(3), vec2(0.0), vec2(f(0), f(1))-vec2(1.001));
        let q = vec2<u32>(s); let t = fract(s); let j = q.y*width()+q.x;
        let seen = rgb(source[j])*((1.0-t.x)*(1.0-t.y)) + rgb(source[j+1u])*(t.x*(1.0-t.y))
            + rgb(source[j+width()])*((1.0-t.x)*t.y) + rgb(source[j+width()+1u])*(t.x*t.y);
        color = floor(color+(seen-color)*clamp(f(2), 0.0, 1.0)+vec3(0.5));
    }
    let c = vec3<u32>(clamp(color, vec3(0.0), vec3(255.0)));
    output[i] = (auxiliary[c.x]<<16u) | (auxiliary[c.y]<<8u) | auxiliary[c.z];
}

@compute @workgroup_size(8, 8)
fn orbs(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let i = id.y*width()+id.x;
    if f(2) < 0.00001 { put(i, vec2(0.0)); return; }
    let xs = light(id.x); let ys = light(width()+id.y);
    var u = xs.x+ys.y; var v = ys.x+xs.y;
    let dx = u-f(4); let dy = v-f(5); let stir = f(6)*0.12/(1.0+(dx*dx+dy*dy)*18.0);
    u += dy*stir; v -= dx*stir;
    var density = 0.0;
    for (var j = 0u; j < u32(f(8)); j++) {
        let l = bitcast<vec4<f32>>(vec4(auxiliary[j*4u], auxiliary[j*4u+1u], auxiliary[j*4u+2u], auxiliary[j*4u+3u]));
        let x = (u-l.x)*l.w; let y = (v-l.y)/l.w;
        density += l.z/(x*x+y*y+l.z*0.16);
    }
    let edge = 1.0/(1.0+(density-1.0)*(density-1.0)*220.0);
    let inner = clamp((density-1.0)*2.0, 0.0, 1.0);
    let fold = 1.0/(1.0+(density-1.75)*(density-1.75)*80.0);
    let core = 1.0/(1.0+(density-2.65)*(density-2.65)*45.0);
    let e = (edge*2.8+inner*0.16+f(3)*(fold*0.8+core*0.4))*f(2);
    let hue = f(7)+u*0.8-v*0.55+(min(density,3.0)-1.0)*0.12;
    put(i, vec2(e, e*clamp(hue,0.0,1.0)));
}

@compute @workgroup_size(8, 8)
fn mandala(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let q = vec2<f32>(id.xy)+vec2(0.5)-vec2(f(0),f(1))*0.5;
    let a = atan2(q.x, -q.y)/f(3);
    let bounce = 1.0-abs((a-2.0*floor(a/2.0))-1.0);
    let angle = bounce*f(3); let dx = cos(angle); let dy = sin(angle);
    var r = length(q)*f(6);
    if r > f(5) { r = max(2.0*f(5)-r, 0.0); }
    let s = vec2(f(4))+r*vec2(dx*f(8)-dy*f(7), dx*f(7)+dy*f(8));
    let v = sharp(s, 1.0, u32(f(2)), u32(f(2)));
    output[id.y*width()+id.x] = projection_color(v);
}

fn bounce(t: f32) -> f32 { return abs(t - 2.0*floor((t+1.0)*0.5)); }
fn triangle_weights(x: f32, y: f32) -> vec3<f32> {
    let b = y*(2.0/sqrt(3.0)); let a = x-b*0.5;
    let i = floor(a); let j = floor(b); let fa = a-i; let fb = b-j;
    let n = u32(i-j-3.0*floor((i-j)/3.0));
    var weights = vec3(fa+fb-1.0, 1.0-fb, 1.0-fa);
    if fa+fb < 1.0 { weights = vec3(1.0-fa-fb,fa,fb); }
    var result = vec3(0.0);
    for (var k = 0u; k < 3u; k++) { result[(n+k)%3u] = weights[k]; }
    return result;
}
fn lens(at: u32, pixel: vec2<f32>) -> vec2<f32> {
    let kind = u32(f(at));
    let x = (f(at+1u)*pixel.x-f(at+2u)*pixel.y)*f(at+3u)+f(at+4u);
    let y = (f(at+2u)*pixel.x+f(at+1u)*pixel.y)*f(at+3u)+f(at+5u);
    let v0 = vec2(f(at+8u),f(at+9u)); let v1 = vec2(f(at+10u),f(at+11u));
    let v2 = vec2(f(at+12u),f(at+13u)); let v3 = vec2(f(at+14u),f(at+15u));
    var weights = vec3(0.0); var point = vec2(0.0);
    if kind < 2u {
        weights = triangle_weights(x,y);
        if kind == 1u {
            let a = max(weights.x,max(weights.y,weights.z));
            let c = min(weights.x,min(weights.y,weights.z));
            let b = min(max(weights.x,weights.y), max(min(weights.x,weights.y),weights.z));
            weights = vec3(a-b, 2.0*(b-c), 3.0*c);
        }
        point = weights.x*v0+weights.y*v1+weights.z*v2;
    } else if kind == 2u {
        let a = max(bounce(x),bounce(y)); let b = min(bounce(x),bounce(y));
        point = (1.0-a)*v0+(a-b)*v1+b*v2;
    } else { point = v0+bounce(x)*(v1-v0)+bounce(y)*(v3-v0); }
    return sharp(point,1.0,u32(f(2)),u32(f(2)));
}
@compute @workgroup_size(8, 8)
fn kaleidoscope(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let shift = vec2(bitcast<f32>(auxiliary[id.y]), bitcast<f32>(auxiliary[height()+id.x]));
    let pixel = vec2<f32>(id.xy)+vec2(0.5)-vec2(f(4),f(5))+shift;
    var value = lens(8u,pixel);
    if f(6) > 0.0 { value = mix(lens(24u,pixel),value,f(3)); }
    output[id.y*width()+id.x] = projection_color(value);
}

struct Ring { phase: vec2<f32>, amp: f32, center: vec2<f32> }
fn read_ring(i: u32) -> Ring {
    let at = i*5u;
    return Ring(bitcast<vec2<f32>>(vec2(source[at],source[at+1u])), bitcast<f32>(source[at+2u]),
        bitcast<vec2<f32>>(vec2(source[at+3u],source[at+4u])));
}
fn ring(d: f32) -> Ring {
    let last = u32(f(2))-1u; let i = min(u32(d),last);
    if i == last { return read_ring(i); }
    let a = read_ring(i); let b = read_ring(i+1u); let k = d-f32(i);
    return Ring(mix(a.phase,b.phase,k),mix(a.amp,b.amp,k),mix(a.center,b.center,k));
}
@compute @workgroup_size(8, 8)
fn gravity(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= width() || id.y >= height() { return; }
    let pixel = vec2<f32>(id.xy); var point = pixel;
    for (var i = 0u; i < u32(f(3)); i++) {
        let center = bitcast<vec2<f32>>(vec2(auxiliary[i*3u],auxiliary[i*3u+1u]));
        let delta = pixel-center; let k = bitcast<f32>(auxiliary[i*3u+2u])/(dot(delta,delta)+1.0);
        point -= delta*k;
    }
    let now = vec2(f(4),f(5)); let stretch = vec2(1.0,f(6)); var origin = now;
    if f(7) > 0.0 { origin = ring(length((point-now)*stretch)).center; }
    let delta = (point-origin)*stretch; let d2 = dot(delta,delta); let d = sqrt(d2);
    var height = 0.0; var amp = read_ring(0u).amp;
    if d2 >= 0.000001 {
        let wave = ring(d);
        height = (wave.phase.x*(delta.x*delta.x-delta.y*delta.y)+wave.phase.y*2.0*delta.x*delta.y)/d2;
        amp = wave.amp;
    }
    let near = min(d*f(8),1.0); let crest = 0.5+0.5*height; let band = crest*crest*crest;
    let e = amp*near*near*(3.0-2.0*near)*(0.22+1.7*band)/(1.0+d*f(9));
    let haze_delta = (point-now)*stretch;
    let glow = f(11)/(1.0+dot(haze_delta,haze_delta)*f(13));
    let hue = 0.18+band*(0.32+0.55*sqrt(min(d*f(10),1.0))); let total = e+glow;
    var eu = 0.0;
    if total > 0.000001 { eu = total*clamp((e*hue+glow*f(12))/total,0.0,1.0); }
    put(id.y*width()+id.x,vec2(total,eu));
}
