// Background visualizer: three layers of glowing silk and drifting motes that follow the music and breathe
// slowly when nothing plays. Chosen design, see docs/DESIGN.md. What drives it (audiofeatures.ts):
//   the river   the last ~5 seconds of music (beats, bass, loudness) travel across the silk as one current, so
//               what you heard a moment ago is still flowing downstream (beats never move or flash the screen)
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
  const FLOW = 96, FLOW_RATE = 20; // samples, samples per second: ~4.8 s of music across the scene
  const U = {
    uTime: { value: 0 }, uBass: { value: 0 }, uBody: { value: 0 }, uAir: { value: 0 }, uIntensity: { value: 0.3 }, uRise: { value: 0 },
    uFlow: { value: Array.from({ length: FLOW / 4 }, () => new THREE.Vector4()) },
    uVoice: { value: Array.from({ length: FLOW / 4 }, () => new THREE.Vector4()) },
    uPhase: { value: 0 }, uSnare: { value: 0 }, uHat: { value: 0 }, uVocal: { value: 0 }, uPitch: { value: 0.5 }, uPR: { value: DPR },
  };
  const PAL = 'vec3 pal(float t){return .55+.45*cos(6.2831*(t+vec3(.0,.2,.42)));}';
  // The river: FLOW samples of the music, newest first, packed four to a vec4. Read with smooth
  // interpolation and a moving phase, so the current glides instead of stepping.
  const RIVER = `uniform vec4 uFlow[${FLOW / 4}],uVoice[${FLOW / 4}];uniform float uPhase;
    float pick(vec4 v,int j){return j==0?v.x:j==1?v.y:j==2?v.z:v.w;}
    float at(bool voice,float i){int k=int(clamp(i,0.,${FLOW - 1}.));vec4 v=voice?uVoice[k/4]:uFlow[k/4];return pick(v,k-(k/4)*4);}
    float river(bool voice,float x01){float i=x01*${FLOW - 2}.+uPhase;float a=at(voice,floor(i)),b=at(voice,floor(i)+1.);float f=fract(i);return mix(a,b,f*f*(3.-2.*f));}`;

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
        vertexShader: `uniform float uTime,uBass,uBody,uAir,uIntensity,uRise,uSnare,uHat,uVocal,uPitch,uPR;attribute float aLayer;varying vec3 vCol;varying float vA;${PAL}${RIVER}
        void main(){
          vec3 p=position;float t=uTime,ph=aLayer*2.1,r=length(p.xz);
          float h=sin(p.x*.16+t*.35+ph)*1.6+sin(p.z*.23-t*.28+ph*.7)*1.1+sin((p.x-p.z)*.09+t*.2)*1.8;
          h*=.35+uIntensity*1.15+uRise*.5+uBody*.25;
          // the current: music flows downstream along x, strongest in the middle of the stream
          float x01=clamp((p.x+40.)/80.,0.,1.);
          float ring=river(false,x01)*exp(-p.z*p.z*.0035)*(1.-x01*.35);
          h+=ring*3.4*(.85+.15*sin(p.z*.35+t*.6+aLayer));
          h+=sin(p.x*.9+t*3.1)*sin(p.z*.8-t*2.4)*(uAir*.6+uHat*.9);
          float voice=aLayer==1.?river(true,x01)*exp(-abs(p.z)*.09)*(1.3+sin(p.x*.14-t*.8)*.7):0.;
          h+=voice*2.2;
          p.y=h+(aLayer-1.)*3.4-2.;
          vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
          vec3 base=pal(p.x*.012+aLayer*.17+t*.018+h*.05+uBody*.25);
          vec3 sing=pal(uPitch*.75+.05);
          vCol=mix(base,sing,clamp(voice*.55,0.,.8))*(.25+.7*smoothstep(-3.,3.,h)+uIntensity*.55+ring*.5);
          vA=smoothstep(40.,24.,abs(position.x))*smoothstep(25.,12.,abs(position.z))*(.4+uIntensity*.6)*(aLayer==1.?1.:.65);
          gl_PointSize=clamp((1.3+max(h,0.)*.35+uAir*1.6+uHat*1.4+voice*.8)*(260./-mv.z)*uPR,1.,14.);
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

  let t = 0, last = performance.now(), listenFailed = false, camDist = 24, speed = 0.6;
  let flowIn = 0, voiceIn = 0, flowPhase = 0;
  const flowBuf = new Float32Array(FLOW), voiceBuf = new Float32Array(FLOW);
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
    // what enters the river now: beats and bass as swells, carried on the general loudness
    const inFlow = Math.min(1.4, f.level * 0.45 + f.kick * 0.55 + f.bass * 0.35 + f.snare * 0.2);
    flowIn += (inFlow - flowIn) * Math.min(1, dt * (inFlow > flowIn ? 14 : 5));
    voiceIn += (f.vocal - voiceIn) * Math.min(1, dt * 5);
    flowPhase += dt * FLOW_RATE;
    while (flowPhase >= 1) {
      flowPhase -= 1;
      flowBuf.copyWithin(1, 0);
      voiceBuf.copyWithin(1, 0);
      flowBuf[0] = calm ? flowIn * 0.4 : flowIn;
      voiceBuf[0] = voiceIn;
    }
    for (let i = 0; i < FLOW / 4; i++) {
      U.uFlow.value[i].set(flowBuf[i * 4], flowBuf[i * 4 + 1], flowBuf[i * 4 + 2], flowBuf[i * 4 + 3]);
      U.uVoice.value[i].set(voiceBuf[i * 4], voiceBuf[i * 4 + 1], voiceBuf[i * 4 + 2], voiceBuf[i * 4 + 3]);
    }
    // the newest sample sits at the upstream edge; the phase moves everything smoothly between samples
    U.uPhase.value = -flowPhase;
    // speed follows the section, eased over seconds so it never lurches
    speed += ((0.55 + f.intensity * 0.9 + f.body * 0.2) - speed) * Math.min(1, dt * 0.5);
    t += dt * (calm ? 0.25 : 1) * speed;
    U.uTime.value = t;
    U.uBass.value = f.bass;
    U.uBody.value = f.body;
    U.uAir.value = f.air;
    U.uIntensity.value = f.intensity;
    U.uRise.value = f.rise;
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
