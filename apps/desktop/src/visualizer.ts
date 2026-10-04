// Background visualizer: three layers of glowing silk and drifting motes over water. Chosen design, see
// docs/DESIGN.md. The music drops into the water and the ripples do the rest (audiofeatures.ts listens).
// Each kind of sound has its own water and its own fixed color, so a hit lights only its color as it spreads:
//   low  (ember)   kick drum: a big drop near the center; bass keeps a slow pulse there
//   high (frost)   snare: a drop somewhere on the surface; hi-hats: small droplets
//   voice (gold → rose by pitch)   a drifting source sending out rings while it sings
// Each drop starts a new wave that spreads, passes through the others, and fades as it fans out; nothing makes
// the whole picture jump. Quiet sections settle the water; loud ones keep it busy. Slow base swells keep the
// silk alive between songs.
import * as THREE from 'three';
import { features, listen } from './audiofeatures';

// The water: a grid matching the silk (x −40..40, z −25..25), simulated with the wave equation.
const GW = 200, GH = 125;
const C2 = 0.3; // wave speed² (cells per step), below the 0.5 stability limit
const STEPS_PER_SEC = 60;

class Water {
  cur = new Float32Array(GW * GH);
  prev = new Float32Array(GW * GH);
  next = new Float32Array(GW * GH);
  /** How much each cell keeps per step: waves fade over a few seconds, and soak in near the edges. */
  keep = new Float32Array(GW * GH);
  constructor() {
    for (let y = 0; y < GH; y++)
      for (let x = 0; x < GW; x++) {
        const edge = Math.min(x, y, GW - 1 - x, GH - 1 - y);
        this.keep[y * GW + x] = edge < 14 ? 0.9935 - (14 - edge) * 0.004 : 0.9935;
      }
  }
  /** Drops something in at (x, z) in scene units: a smooth bump that becomes an outgoing ring. */
  drop(sx: number, sz: number, radius: number, amp: number) {
    const cx = ((sx + 40) / 80) * (GW - 1), cy = ((sz + 25) / 50) * (GH - 1);
    const r = Math.ceil(radius * 2.5);
    for (let y = Math.max(1, Math.floor(cy - r)); y < Math.min(GH - 1, cy + r); y++)
      for (let x = Math.max(1, Math.floor(cx - r)); x < Math.min(GW - 1, cx + r); x++) {
        const d2 = ((x - cx) ** 2 + (y - cy) ** 2) / (radius * radius);
        // added to both steps, so it starts at rest and spreads as a ring (not thrown in with a push)
        if (d2 < 6.25) {
          const v = amp * Math.exp(-d2);
          this.cur[y * GW + x] += v;
          this.prev[y * GW + x] += v;
        }
      }
  }
  step() {
    const { cur, prev, next, keep } = this;
    for (let y = 1; y < GH - 1; y++) {
      const row = y * GW;
      for (let x = 1; x < GW - 1; x++) {
        const i = row + x;
        const lap = cur[i - 1] + cur[i + 1] + cur[i - GW] + cur[i + GW] - 4 * cur[i];
        next[i] = (2 * cur[i] - prev[i] + C2 * lap) * keep[i];
      }
    }
    this.prev = cur;
    this.cur = next;
    this.next = prev;
  }
}

export function startVisualizer(canvas: HTMLCanvasElement) {
  let renderer: THREE.WebGLRenderer;
  try {
    renderer = new THREE.WebGLRenderer({ canvas, antialias: false, powerPreference: 'high-performance' });
  } catch {
    canvas.hidden = true;
    return;
  }
  const DPR = Math.min(window.devicePixelRatio || 1, 1.5);
  const calm = matchMedia('(prefers-reduced-motion: reduce)').matches;
  renderer.setPixelRatio(DPR);
  renderer.setClearColor(0x0b0d24, 1);
  const scene = new THREE.Scene();
  const cam = new THREE.PerspectiveCamera(55, 1, 0.1, 200);

  const low = new Water(), high = new Water(), voiceW = new Water();
  const waveData = new Float32Array(GW * GH * 4);
  const waveTex = new THREE.DataTexture(waveData, GW, GH, THREE.RGBAFormat, THREE.FloatType);
  waveTex.minFilter = waveTex.magFilter = THREE.NearestFilter;
  waveTex.needsUpdate = true;

  const U = {
    uTime: { value: 0 }, uIntensity: { value: 0.3 }, uVocal: { value: 0 }, uPitch: { value: 0.5 }, uHat: { value: 0 },
    uWave: { value: waveTex }, uPR: { value: DPR },
  };
  const PAL = 'vec3 pal(float t){return .55+.45*cos(6.2831*(t+vec3(.0,.2,.42)));}';

  const NX = 220, NZ = 130, L = 3, N = NX * NZ * L;
  const pos = new Float32Array(N * 3), lay = new Float32Array(N);
  let k = 0;
  for (let l = 0; l < L; l++)
    for (let i = 0; i < NX; i++)
      for (let j = 0; j < NZ; j++) {
        pos[k * 3] = (i / (NX - 1) - 0.5) * 80;
        pos[k * 3 + 2] = (j / (NZ - 1) - 0.5) * 50;
        lay[k] = l;
        k++;
      }
  const g = new THREE.BufferGeometry();
  g.setAttribute('position', new THREE.BufferAttribute(pos, 3));
  g.setAttribute('aLayer', new THREE.BufferAttribute(lay, 1));
  scene.add(
    new THREE.Points(
      g,
      new THREE.ShaderMaterial({
        uniforms: U, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
        vertexShader: `uniform float uTime,uIntensity,uVocal,uPitch,uPR;uniform sampler2D uWave;attribute float aLayer;varying vec3 vCol;varying float vA;${PAL}
        void main(){
          vec3 p=position;float t=uTime,ph=aLayer*2.1;
          // slow base swells (the same whether music plays or not)
          float h=(sin(p.x*.16+t*.35+ph)*1.6+sin(p.z*.23-t*.28+ph*.7)*1.1+sin((p.x-p.z)*.09+t*.2)*1.8)*(.35+uIntensity*.35);
          // the water: low, high and voice ripples; every layer rides them, the upper ones a little softer
          vec2 uv=vec2((p.x+40.)/80.,(p.z+25.)/50.);
          vec3 wv=texture2D(uWave,uv).rgb;
          float w=wv.r+wv.g+wv.b;
          h+=w*(3.4-abs(aLayer-1.)*.8);
          p.y=h+(aLayer-1.)*3.4-2.;
          vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
          // calm colors drift on their own; each sound adds only its own color where its ripples are
          vec3 calmC=pal(p.x*.012+aLayer*.17+t*.018)*(.22+.5*smoothstep(-3.,3.,h)+uIntensity*.3);
          vec3 ember=vec3(1.,.42,.16),frost=vec3(.55,.9,1.),voiceC=mix(vec3(1.,.78,.3),vec3(1.,.38,.62),uPitch);
          vec3 lit=ember*min(abs(wv.r)*2.2,1.6)+frost*min(abs(wv.g)*2.6,1.6)+voiceC*min(abs(wv.b)*3.,1.6);
          vCol=calmC+lit;
          vA=smoothstep(40.,24.,abs(position.x))*smoothstep(25.,12.,abs(position.z))*(.45+uIntensity*.45)*(aLayer==1.?1.:.65);
          gl_PointSize=clamp((1.3+max(h,0.)*.3+min((abs(wv.r)+abs(wv.g)+abs(wv.b))*1.8,2.))*(260./-mv.z)*uPR,1.,14.);
        }`,
        fragmentShader: `varying vec3 vCol;varying float vA;void main(){float d=length(gl_PointCoord-.5);gl_FragColor=vec4(vCol,smoothstep(.5,.05,d)*vA);}`,
      }),
    ),
  );

  const M = 1800, mp = new Float32Array(M * 3), ms = new Float32Array(M);
  for (let i = 0; i < M; i++) {
    mp[i * 3] = (Math.random() - 0.5) * 90;
    mp[i * 3 + 1] = Math.random() * 22 - 6;
    mp[i * 3 + 2] = (Math.random() - 0.5) * 60;
    ms[i] = Math.random();
  }
  const mg = new THREE.BufferGeometry();
  mg.setAttribute('position', new THREE.BufferAttribute(mp, 3));
  mg.setAttribute('aSeed', new THREE.BufferAttribute(ms, 1));
  scene.add(
    new THREE.Points(
      mg,
      new THREE.ShaderMaterial({
        uniforms: U, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
        vertexShader: `uniform float uTime,uHat,uIntensity,uVocal,uPitch,uPR;attribute float aSeed;varying vec3 vCol;varying float vA;${PAL}
        void main(){
          vec3 p=position;p.y=mod(p.y+uTime*.35*(.3+aSeed)+6.,22.)-6.;p.x+=sin(uTime*.2+aSeed*30.)*1.5;
          float tw=.5+.5*sin(uTime*(1.+aSeed*2.)+aSeed*40.);
          float spark=step(.88,fract(aSeed*91.7+uTime*.05))*uHat;
          vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
          vCol=pal(aSeed*.6+uTime*.02+.3)*(.6+tw*.8)+vec3(.55,.9,1.)*spark*1.4;
          vA=(.2+tw*.4+spark*.35)*(.55+uIntensity*.4);
          gl_PointSize=clamp((1.+aSeed*2.2+spark*2.)*(200./-mv.z)*uPR,1.,10.);
        }`,
        fragmentShader: `varying vec3 vCol;varying float vA;void main(){float d=length(gl_PointCoord-.5);gl_FragColor=vec4(vCol,smoothstep(.5,0.,d)*vA);}`,
      }),
    ),
  );

  const size = () => {
    renderer.setSize(innerWidth, innerHeight, false);
    cam.aspect = innerWidth / innerHeight;
    cam.updateProjectionMatrix();
  };
  addEventListener('resize', size);
  size();

  let t = 0, last = performance.now(), listenFailed = false, camDist = 24, speed = 0.6;
  let stepClock = 0, voicePhase = 0, voiceX = 0, intensity = 0.3, vocal = 0, hatGlow = 0;
  const prevHit = { kick: 0, snare: 0, hat: 0 };
  const rnd = (a: number) => (Math.random() * 2 - 1) * a;
  const frame = (now: number) => {
    requestAnimationFrame(frame);
    if (document.hidden) {
      last = now;
      return;
    }
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    // listening must never stop the picture: on any error it carries on without the music
    try {
      listen(dt);
    } catch (e) {
      if (!listenFailed) console.error('visualizer listening failed', e);
      listenFailed = true;
    }
    const f = features;
    const soft = calm ? 0.35 : 1;

    // drops: one per new hit (a rise in the hit signal), each starting its own wave
    const rose = (k: 'kick' | 'snare' | 'hat') => {
      const r = f[k] > prevHit[k] + 0.4;
      prevHit[k] = f[k];
      return r;
    };
    if (rose('kick')) low.drop(rnd(6), rnd(4), 5.5, -(1.7 + f.bass * 1.8) * soft);
    if (rose('snare')) high.drop(rnd(24), rnd(13), 3.2, -1.4 * soft);
    if (rose('hat')) high.drop(rnd(34), rnd(20), 1.8, -0.45 * soft);

    // the voice: a source drifting slowly along the middle, sending rings while it sings (higher = tighter)
    vocal += (f.vocal - vocal) * Math.min(1, dt * 3);
    voiceX = Math.sin(t * 0.07) * 16;
    voicePhase += dt * Math.PI * 2 * (0.9 + f.pitch * 2.2);

    stepClock += dt * STEPS_PER_SEC;
    let n = 0;
    while (stepClock >= 1 && n < 4) {
      stepClock -= 1;
      n++;
      if (vocal > 0.15) voiceW.drop(voiceX, 0, 2.4, Math.sin(voicePhase) * vocal * 0.06 * soft);
      // the bass: a slow pulse at the center while it plays
      if (f.bass > 0.2) low.drop(0, 0, 6, Math.sin(voicePhase * 0.25) * f.bass * 0.025 * soft);
      low.step();
      high.step();
      voiceW.step();
    }
    if (n === 4) stepClock = 0; // a long pause (window hidden): don't race to catch up
    for (let i = 0, a = low.cur, b = high.cur, c = voiceW.cur; i < a.length; i++) {
      waveData[i * 4] = a[i];
      waveData[i * 4 + 1] = b[i];
      waveData[i * 4 + 2] = c[i];
    }
    waveTex.needsUpdate = true;

    // slow things: the section's loudness eases in over seconds
    intensity += (f.intensity - intensity) * Math.min(1, dt * 0.6);
    hatGlow += (f.air - hatGlow) * Math.min(1, dt * 2);
    speed += ((0.5 + intensity * 0.6) - speed) * Math.min(1, dt * 0.5);
    t += dt * (calm ? 0.25 : 1) * speed;
    U.uTime.value = t;
    U.uIntensity.value = intensity;
    U.uVocal.value = vocal;
    U.uPitch.value = f.pitch;
    U.uHat.value = hatGlow;

    const a = calm ? 0.6 : t * 0.035;
    // the camera only drifts; a loud section brings it in slowly
    camDist += (24 - intensity * 4 - camDist) * Math.min(1, dt * 0.4);
    cam.position.set(Math.sin(a) * camDist, 7 + Math.sin(t * 0.11) * 1.6, Math.cos(a) * camDist);
    cam.lookAt(0, -1, 0);
    renderer.render(scene, cam);
  };
  requestAnimationFrame(frame);
}
