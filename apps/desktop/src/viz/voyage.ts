// Voyage: flying through a starfield past a nebula. Each kick sends out an ember shell of light that keeps
// expanding while newer ones start; snares send thinner frost shells; the voice swells a gold → rose glow in
// the nebula; hi-hats make stars glint; louder sections speed up the flight (smoothly).
import * as THREE from 'three';
import { Ease, GLSL_NOISE } from './noise';
import { GLSL_COLORS, Ripples, type Frame, type MakeScene } from './types';

const STARS = 5000, SHELL = 700, SHELLS = 8;

export const makeVoyage: MakeScene = (renderer) => {
  const PR = Math.min(window.devicePixelRatio || 1, 1.5);
  const scene = new THREE.Scene();
  const cam = new THREE.PerspectiveCamera(70, 1, 0.1, 400);
  const kicks = new Ripples(SHELLS), snares = new Ripples(SHELLS);
  const U = {
    uTime: { value: 0 }, uTravel: { value: 0 }, uHat: { value: 0 }, uVoice: { value: 0 }, uPitch: { value: 0.5 }, uLow: { value: 0 },
    uKickAge: { value: kicks.ages }, uKickAmp: { value: kicks.amps }, uSnareAge: { value: snares.ages }, uSnareAmp: { value: snares.amps },
    uPR: { value: PR }, uRes: { value: new THREE.Vector2(1, 1) },
  };

  // the nebula, behind everything
  const bgScene = new THREE.Scene();
  const bgCam = new THREE.OrthographicCamera(-1, 1, 1, -1, 0, 1);
  const bgMat = new THREE.ShaderMaterial({
    uniforms: U, depthWrite: false,
    vertexShader: 'varying vec2 vUv;void main(){vUv=uv;gl_Position=vec4(position.xy,0.,1.);}',
    fragmentShader: `precision highp float;varying vec2 vUv;uniform vec2 uRes;uniform float uTime,uVoice,uPitch,uLow;${GLSL_NOISE}${GLSL_COLORS}
      void main(){vec2 p=(vUv-.5)*vec2(uRes.x/uRes.y,1.)*2.;
        float n=fbm(p*1.4+vec2(uTime*.02,-uTime*.015));float m=fbm(p*2.6-vec2(uTime*.03,0.)+n);
        vec3 col=vec3(.01,.012,.035)+vec3(.12,.06,.22)*m*m*(.6+uLow*.5);
        col+=voiceColor(uPitch)*smoothstep(.45,.9,n*m*1.6)*(.05+uVoice*.45);
        gl_FragColor=vec4(col,1.);}`,
  });
  bgScene.add(new THREE.Mesh(new THREE.PlaneGeometry(2, 2), bgMat));

  // stars in a long tube, flowing toward the camera
  const sp = new Float32Array(STARS * 3), seed = new Float32Array(STARS);
  for (let i = 0; i < STARS; i++) {
    const a = Math.random() * Math.PI * 2, r = 4 + Math.random() * 60;
    sp[i * 3] = Math.cos(a) * r;
    sp[i * 3 + 1] = Math.sin(a) * r;
    sp[i * 3 + 2] = -Math.random() * 300;
    seed[i] = Math.random();
  }
  const sg = new THREE.BufferGeometry();
  sg.setAttribute('position', new THREE.BufferAttribute(sp, 3));
  sg.setAttribute('aSeed', new THREE.BufferAttribute(seed, 1));
  scene.add(new THREE.Points(sg, new THREE.ShaderMaterial({
    uniforms: U, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
    vertexShader: `uniform float uTravel,uTime,uHat,uPR;attribute float aSeed;varying float vA;varying vec3 vC;${GLSL_COLORS}
      void main(){vec3 p=position;p.z=mod(p.z+uTravel,300.)-300.;
        vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
        float glint=step(.9,fract(aSeed*53.+uTime*.25))*uHat;
        vA=smoothstep(-300.,-200.,p.z)*(.35+aSeed*.5+glint);
        vC=mix(vec3(.8,.85,1.),FROST,glint);
        gl_PointSize=clamp((1.+aSeed*1.8+glint*2.5)*(120./-mv.z)*uPR,1.,9.);}`,
    fragmentShader: 'varying float vA;varying vec3 vC;void main(){float d=length(gl_PointCoord-.5);gl_FragColor=vec4(vC,smoothstep(.5,0.,d)*vA);}',
  })));

  // shells: directions on a sphere, each copy expanding at its hit's age
  const n = SHELL * SHELLS * 2;
  const dir = new Float32Array(n * 3), which = new Float32Array(n);
  for (let i = 0; i < n; i++) {
    const u = Math.random() * 2 - 1, a = Math.random() * Math.PI * 2, s = Math.sqrt(1 - u * u);
    dir[i * 3] = Math.cos(a) * s;
    dir[i * 3 + 1] = Math.sin(a) * s;
    dir[i * 3 + 2] = u;
    which[i] = Math.floor(i / SHELL); // 0..7 kick shells, 8..15 snare shells
  }
  const hg = new THREE.BufferGeometry();
  hg.setAttribute('position', new THREE.BufferAttribute(dir, 3));
  hg.setAttribute('aWhich', new THREE.BufferAttribute(which, 1));
  scene.add(new THREE.Points(hg, new THREE.ShaderMaterial({
    uniforms: U, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending,
    vertexShader: `uniform float uKickAge[${SHELLS}],uKickAmp[${SHELLS}],uSnareAge[${SHELLS}],uSnareAmp[${SHELLS}],uPR;attribute float aWhich;varying float vA;varying vec3 vC;${GLSL_COLORS}
      void main(){int w=int(aWhich);float age=99.,amp=0.;bool snare=w>=${SHELLS};int k=snare?w-${SHELLS}:w;
        for(int i=0;i<${SHELLS};i++){if(i==k){age=snare?uSnareAge[i]:uKickAge[i];amp=snare?uSnareAmp[i]:uKickAmp[i];}}
        vec3 p=position*(age*(snare?16.:11.))+vec3(0.,0.,-40.);
        vec4 mv=modelViewMatrix*vec4(p,1.);gl_Position=projectionMatrix*mv;
        vA=amp*exp(-age*(snare?1.6:1.1))*(snare?.55:.8);vC=snare?FROST:EMBER;
        gl_PointSize=clamp((snare?1.2:1.8)*(140./-mv.z)*uPR,1.,6.);}`,
    fragmentShader: 'varying float vA;varying vec3 vC;void main(){float d=length(gl_PointCoord-.5);gl_FragColor=vec4(vC,smoothstep(.5,0.,d)*vA);}',
  })));

  const hat = new Ease(8, 2), voice = new Ease(3, 1.2), pitch = new Ease(2, 2), low = new Ease(3, 1), speed = new Ease(0.5, 0.5);
  let t = 0, travel = 0;
  return {
    frame({ dt, f, hit, calm, width, height }: Frame) {
      const k = calm ? 0.3 : 1;
      t += dt * k;
      if (cam.aspect !== width / height) {
        cam.aspect = width / height;
        cam.updateProjectionMatrix();
      }
      if (hit.kick) kicks.add((0.6 + f.bass * 0.4) * (calm ? 0.4 : 1));
      if (hit.snare) snares.add(calm ? 0.3 : 0.7);
      kicks.step(dt);
      snares.step(dt);
      travel += dt * k * speed.to(6 + f.intensity * 22, dt);
      U.uTime.value = t;
      U.uTravel.value = travel;
      U.uHat.value = hat.to(f.hat, dt);
      U.uVoice.value = voice.to(f.vocal, dt);
      U.uPitch.value = pitch.to(f.pitch, dt);
      U.uLow.value = low.to(f.bass, dt);
      (U.uRes.value as THREE.Vector2).set(width, height);
      cam.position.set(Math.sin(t * 0.05) * 2, Math.cos(t * 0.04) * 1.5, 0);
      cam.lookAt(0, 0, -50);
      renderer.autoClear = false;
      renderer.clear();
      renderer.render(bgScene, bgCam);
      renderer.render(scene, cam);
      renderer.autoClear = true;
    },
    dispose() {
      for (const o of [...scene.children, ...bgScene.children]) {
        const m = o as THREE.Mesh;
        m.geometry.dispose();
        (m.material as THREE.Material).dispose();
      }
    },
  };
};
