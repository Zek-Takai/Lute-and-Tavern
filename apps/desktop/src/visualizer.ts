// Background visualizer: three layers of glowing silk and drifting motes that follow the music and breathe
// slowly when nothing plays. Chosen design, see docs/DESIGN.md. What drives it (audiofeatures.ts):
//   kick drum   a ring of light rolls out from the center      snare   points along the silk grow
//   hi-hats     the surface shimmers and motes sparkle          bass    the center swells
//   voice       a ribbon rises along the middle layer and takes the color of the singer's pitch
//   section     quiet verses lie low and slow; a big chorus lifts everything, speeds it up, brings the camera in
//   build-up    the waves grow taller as the music rises
import * as THREE from 'three';
import { features, listen } from './audiofeatures';

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
  const U = {
    uTime: { value: 0 }, uBass: { value: 0 }, uBody: { value: 0 }, uAir: { value: 0 }, uIntensity: { value: 0.3 }, uRise: { value: 0 },
    uKickAge: { value: 9 }, uKickAmp: { value: 0 }, uSnare: { value: 0 }, uHat: { value: 0 }, uVocal: { value: 0 }, uPitch: { value: 0.5 }, uPR: { value: DPR },
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
        vertexShader: `uniform float uTime,uBass,uBody,uAir,uIntensity,uRise,uKickAge,uKickAmp,uSnare,uHat,uVocal,uPitch,uPR;attribute float aLayer;varying vec3 vCol;varying float vA;${PAL}
        void main(){
          vec3 p=position;float t=uTime,ph=aLayer*2.1,r=length(p.xz);
          float h=sin(p.x*.16+t*.35+ph)*1.6+sin(p.z*.23-t*.28+ph*.7)*1.1+sin((p.x-p.z)*.09+t*.2)*1.8;
          h*=.35+uIntensity*1.15+uRise*.5+uBody*.25;
          h+=sin(r*.55-t*2.6)*uBass*2.4*exp(-r*.035);
          float rd=(r-uKickAge*34.)*.33;
          float ring=exp(-rd*rd)*uKickAmp*exp(-uKickAge*2.2);
          h+=ring*3.;
          h+=sin(p.x*.9+t*3.1)*sin(p.z*.8-t*2.4)*(uAir*.6+uHat*.9);
          float voice=aLayer==1.?uVocal*exp(-abs(p.z)*.09)*(1.4+sin(p.x*.14-t*1.1)*1.1):0.;
          h+=voice*2.2;
          p.y=h+(aLayer-1.)*3.4-2.;
          vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
          vec3 base=pal(p.x*.012+aLayer*.17+t*.018+h*.05+uBody*.25);
          vec3 sing=pal(uPitch*.75+.05);
          vCol=mix(base,sing,clamp(voice*.55,0.,.8))*(.25+.7*smoothstep(-3.,3.,h)+uIntensity*.55+ring*.5);
          vA=smoothstep(40.,24.,abs(position.x))*smoothstep(25.,12.,abs(position.z))*(.4+uIntensity*.6)*(aLayer==1.?1.:.65);
          gl_PointSize=clamp((1.3+max(h,0.)*.35+uAir*1.6+uHat*1.4+uSnare*1.2+voice*.8)*(260./-mv.z)*uPR,1.,14.);
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
        vertexShader: `uniform float uTime,uBass,uAir,uHat,uSnare,uIntensity,uVocal,uPitch,uPR;attribute float aSeed;varying vec3 vCol;varying float vA;${PAL}
        void main(){
          vec3 p=position;p.y=mod(p.y+uTime*.35*(.3+aSeed)+6.,22.)-6.;p.x+=sin(uTime*.2+aSeed*30.)*1.5;
          float tw=.5+.5*sin(uTime*(1.+aSeed*2.)+aSeed*40.);
          float spark=step(.82,fract(aSeed*91.7+uTime*.07))*uHat;
          vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
          vCol=mix(pal(aSeed*.6+uTime*.02+.3),pal(uPitch*.75+.05),uVocal*.35)*(.6+tw*.8+spark*1.5);
          vA=(.2+tw*.4+spark*.5)*(.55+uIntensity*.6);
          gl_PointSize=clamp((1.+aSeed*2.2+uAir*1.8+spark*3.)*(200./-mv.z)*uPR,1.,10.);
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

  let t = 0, last = performance.now(), kickAge = 9, kickAmp = 0, lastKick = 0, listenFailed = false, camDist = 24;
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
    // a new kick starts a new ring
    if (f.kick > lastKick + 0.5) {
      kickAge = 0;
      kickAmp = 0.5 + f.bass * 0.5;
    }
    lastKick = f.kick;
    kickAge += dt;
    t += dt * (calm ? 0.25 : 1) * (0.55 + f.intensity * 1.1 + f.body * 0.3);
    U.uTime.value = t;
    U.uBass.value = f.bass;
    U.uBody.value = f.body;
    U.uAir.value = f.air;
    U.uIntensity.value = f.intensity;
    U.uRise.value = f.rise;
    U.uKickAge.value = calm ? 9 : kickAge;
    U.uKickAmp.value = kickAmp;
    U.uSnare.value = calm ? f.snare * 0.3 : f.snare;
    U.uHat.value = f.hat;
    U.uVocal.value = f.vocal;
    U.uPitch.value = f.pitch;
    const a = calm ? 0.6 : t * 0.035;
    // a chorus brings the camera in slowly
    camDist += (24 - f.intensity * 4 - camDist) * Math.min(1, dt * 0.4);
    const dist = camDist;
    // the camera only drifts: no jumps or zooms on beats (the whole screen pulsing is hard on the eyes)
    cam.position.set(Math.sin(a) * dist, 7 + Math.sin(t * 0.11) * 1.6, Math.cos(a) * dist);
    cam.lookAt(0, -1, 0);
    renderer.render(scene, cam);
  };
  requestAnimationFrame(frame);
}
