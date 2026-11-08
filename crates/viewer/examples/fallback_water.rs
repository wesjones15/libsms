//! Generated cylinders and HUD shapes, no original assets or cue/model IDs.
//! Water spans use the same flight_shape as textured FLUDD; emissions are live sims.
use libsms::{Sim,Params,AnimTable,Pad,Host,Event,GroundHit,FlatHost,Vec3 as GVec3,WaterEmit};
use libsms::water::WaterManager;
use libsms_present::water::flight_shape;
use macroquad::prelude::*;
#[derive(Default)]struct World{emits:Vec<WaterEmit>}
impl Host for World{fn ground(&self,p:GVec3)->Option<GroundHit>{FlatHost::default().ground(p)}fn event(&mut self,e:Event){if let Event::WaterEmit(w)=e{self.emits.push(w);}}}
fn config()->Conf{Conf{window_title:"Primitive FLUDD water and gauge".into(),window_width:1100,window_height:760,..Default::default()}}
fn point(v:GVec3)->Vec3{vec3(v.x,v.y,v.z)}
fn segment(a:Vec3,b:Vec3,r:f32){
 let axis=(b-a).normalize_or_zero();if axis.length_squared()<0.5{return;}
 let x=axis.cross(if axis.y.abs()>0.99{Vec3::X}else{Vec3::Y}).normalize();let y=axis.cross(x);
 let mut mesh=Mesh{vertices:Vec::new(),indices:Vec::new(),texture:None};
 for i in 0..=12{let angle=i as f32*std::f32::consts::TAU/12.;let side=(x*angle.cos()+y*angle.sin())*r;
  for p in [a+side,b+side]{mesh.vertices.push(Vertex::new(p.x,p.y,p.z,0.,0.,BLUE));}
  if i<12{let n=i*2;mesh.indices.extend([n,n+1,n+2,n+1,n+3,n+2]);}}
 draw_mesh(&mesh);
}
#[macroquad::main(config)]async fn main(){
 let out=std::env::args().nth(1).expect("output directory");std::fs::create_dir_all(&out).unwrap();
 let p=Params::builtin();let mut sims=[Sim::new(p.clone(),AnimTable::builtin(),GVec3::new(-180.,0.,0.),0),Sim::new(p.clone(),AnimTable::builtin(),GVec3::new(180.,0.,0.),0)];
 for sim in &mut sims{sim.mario.give_fludd();}
 let mut worlds=[World::default(),World::default()];let mut waters=[WaterManager::new(&p),WaterManager::new(&p)];
 for frame in 0..=91{
  for i in 0..2{for tick in 0..2{
   let press=if i==0{255}else if frame<10{12}else{0};let pad=Pad{r_analog:press,..Default::default()};sims[i].advance(&mut worlds[i],&pad,0,1./120.);
   for emit in worlds[i].emits.drain(..){waters[i].emit(&emit);}
   if (frame*2+tick)%4==3{waters[i].step(&FlatHost::default(),sims[i].mario.pos);}
  }}
  clear_background(Color::from_rgba(23,29,40,255));set_camera(&Camera3D{position:vec3(11.,8.,11.),target:vec3(0.,1.,3.),up:Vec3::Y,..Default::default()});draw_grid(20,1.,GRAY,DARKGRAY);
  for i in 0..2{let feet=point(sims[i].mario.pos)*0.01;draw_cylinder(feet+Vec3::Y*0.8,0.35,0.35,1.6,None,RED);
   let muzzle=sims[i].mario.muzzle_world(0);
   for drop in &waters[i].particles{if let Some(shape)=flight_shape(&waters[i],drop,0.,muzzle){let(a,b)=shape.ends();segment(point(a)*0.01,point(b)*0.01,shape.half*0.01);}}
  }
  set_default_camera();draw_text("PROCEDURAL FLUDD | shared flight span, live Spray and short-burst emissions",24.,36.,22.,WHITE);
  draw_text("Left: held Spray. Right: light press, then release. No original assets/effects.",24.,65.,21.,LIGHTGRAY);
  for (i,charge) in [0.,0.5,1.].into_iter().enumerate(){let x=760.+i as f32*110.;let y=570.;let r=44.;draw_circle(x,y,r,DARKGRAY);
   for row in 0..88{let dy=row as f32-r;let span=(r*r-dy*dy).max(0.).sqrt();if dy>r-2.*r*charge{draw_line(x-span,y+dy,x+span,y+dy,1.,ORANGE);}}
   draw_circle_lines(x,y,r,3.,YELLOW);draw_rectangle(x-15.,y-24.,30.,48.,BLACK);draw_rectangle(x-12.,y+21.-42.*0.6,24.,42.*0.6,BLUE);draw_text("R",x-8.,y+8.,25.,WHITE);
  }
  draw_text("Basic HUD: water rectangle, nozzle letter, rising Rocket charge",450.,660.,20.,WHITE);
  if [8,18,30].contains(&frame){get_screen_data().export_png(&format!("{out}/water-{frame}.png"));}next_frame().await;
 }
}
