// Background visualizer: three layers of glowing silk and drifting motes that swell with the music
// and breathe slowly when nothing plays. Chosen design, see docs/DESIGN.md.
import * as THREE from 'three';
import { analyser, playing } from './player';

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
  const U = { uTime: { value: 0 }, uBass: { value: 0 }, uMid: { value: 0 }, uTreble: { value: 0 }, uEnergy: { value: 0 }, uPR: { value: DPR } };
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
        vertexShader: `uniform float uTime,uBass,uMid,uTreble,uEnergy,uPR;attribute float aLayer;varying vec3 vCol;varying float vA;${PAL}
        void main(){
          vec3 p=position;float t=uTime,ph=aLayer*2.1,r=length(p.xz);
          float h=sin(p.x*.16+t*.35+ph)*1.6+sin(p.z*.23-t*.28+ph*.7)*1.1+sin((p.x-p.z)*.09+t*.2)*1.8;
          h*=.55+uEnergy*1.2;
          h+=sin(r*.55-t*2.6)*uBass*3.4*exp(-r*.035);
          h+=sin(p.x*.9+t*3.1)*sin(p.z*.8-t*2.4)*uTreble*1.1;
          p.y=h+(aLayer-1.)*3.4-2.;
          vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
          vCol=pal(p.x*.012+aLayer*.17+t*.018+h*.05+uMid*.35)*(.3+.7*smoothstep(-3.,3.,h)+uEnergy*.7);
          vA=smoothstep(40.,24.,abs(position.x))*smoothstep(25.,12.,abs(position.z))*(.5+uEnergy*.5)*(aLayer==1.?1.:.65);
          gl_PointSize=clamp((1.3+max(h,0.)*.35+uTreble*2.4)*(260./-mv.z)*uPR,1.,14.);
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
        vertexShader: `uniform float uTime,uBass,uTreble,uEnergy,uPR;attribute float aSeed;varying vec3 vCol;varying float vA;${PAL}
        void main(){
          vec3 p=position;p.y=mod(p.y+uTime*.35*(.3+aSeed)+6.,22.)-6.;p.x+=sin(uTime*.2+aSeed*30.)*1.5;
          float tw=.5+.5*sin(uTime*(1.+aSeed*2.)+aSeed*40.);
          vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
          vCol=pal(aSeed*.6+uTime*.02+.3)*(.6+tw*.8);vA=(.25+tw*.45)*(.7+uBass*.8);
          gl_PointSize=clamp((1.+aSeed*2.2+uTreble*3.)*(200./-mv.z)*uPR,1.,10.);
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

  const buf = new Uint8Array(512);
  const lv = { bass: 0, mid: 0, treble: 0, energy: 0 };
  const avg = (a: number, b: number) => {
    let s = 0;
    for (let i = a; i < b; i++) s += buf[i];
    return s / ((b - a) * 255);
  };
  let t = 0, last = performance.now();
  const frame = (now: number) => {
    requestAnimationFrame(frame);
    if (document.hidden) {
      last = now;
      return;
    }
    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    let tg = { bass: 0, mid: 0, treble: 0 };
    if (playing && analyser) {
      analyser.getByteFrequencyData(buf);
      tg = { bass: Math.min(1, Math.pow(avg(1, 7), 1.5) * 1.6), mid: Math.min(1, avg(7, 50) * 1.5), treble: Math.min(1, avg(50, 220) * 2.6) };
    }
    for (const b of ['bass', 'mid', 'treble'] as const) lv[b] += (tg[b] - lv[b]) * (tg[b] > lv[b] ? 0.35 : 0.05);
    lv.energy += (((lv.bass + lv.mid + lv.treble) / 3) * 1.5 - lv.energy) * 0.08;
    t += dt * (calm ? 0.25 : 1) * (1 + lv.energy * 1.4);
    U.uTime.value = t;
    U.uBass.value = lv.bass;
    U.uMid.value = lv.mid;
    U.uTreble.value = lv.treble;
    U.uEnergy.value = Math.min(1, lv.energy);
    const a = calm ? 0.6 : t * 0.035;
    cam.position.set(Math.sin(a) * 24, 7 + Math.sin(t * 0.11) * 1.6, Math.cos(a) * 24);
    cam.lookAt(0, -1, 0);
    cam.fov = 55 - lv.bass * 3;
    cam.updateProjectionMatrix();
    renderer.render(scene, cam);
  };
  requestAnimationFrame(frame);
}
