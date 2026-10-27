//! Sunshine Yoshi wearing Mario's animation, for the standalone viewer.
//! Exact asset: yoshi.szs:yoshi_model.bmd (38 joints), loaded by TYoshi::init
//! in sms-main/src/Player/Yoshi.cpp. Motion: mario.szs:bck/ma_*.bck.
//! Host adaptation: the neck maps onto Mario's torso/head; tail and mouth inherit
//! their authored local transforms. This does not use Yoshi's riding animations.
use std::path::Path;
use glam::{Mat4,Quat,Vec3};
use libsms::{Skeleton,Mario,NozzleKind,WaterEmit,Params};
use libsms_assets::retarget::Retarget;

const ALIASES: &[(&str,&str)] = &[
    ("chn_neck","chn_chest"),
    ("jnt_neck_1","jnt_chest"),
    ("jnt_neck_2","jnt_chest"),
    ("chn_head","jnt_head"),
];

pub struct YoshiSkin {
    rig: Skeleton,
    retarget: Retarget,
    chest: usize,
    chest_align: Mat4,
    mouth: usize,
    head: usize,
    jaw: usize,
    eat: libsms::Clip,
    mouth_frame: std::cell::Cell<Mat4>,
}

/// Viewer adaptation. The front clearance envelope uses the mouth; outside it
/// FLUDD handles the aim. Physical head pitch limits come from NozzleYoshiMouth.prm.
#[derive(Clone,Copy,Debug)]
pub struct SprayRoute {pub mouth:bool,pub yaw:f32,pub pitch:f32}

impl SprayRoute {
    pub fn new(yaw:f32,pitch:f32,nozzle:NozzleKind,params:&Params)->Self {
        let n=params.nozzle(NozzleKind::Yoshi);
        let unit=std::f32::consts::PI/32768.;
        let side=(n.side_angle_max_side as f32*unit).min(60f32.to_radians());
        let down=-n.l_angle_max as f32*unit;
        let up=-n.l_angle_min as f32*unit;
        let yaw=(yaw+std::f32::consts::PI).rem_euclid(std::f32::consts::TAU)-std::f32::consts::PI;
        Self{mouth:nozzle==NozzleKind::Spray && yaw.abs()<=side && pitch>=down && pitch<=up,
            yaw:yaw.clamp(-side,side),pitch:pitch.clamp(-n.l_angle_max as f32*unit,-n.l_angle_min as f32*unit)}
    }
    pub fn for_mario(m:&Mario)->Self {
        let unit=std::f32::consts::PI/32768.;
        let (yaw,pitch)=m.aim.map_or((0.,0.),|a|(a.yaw.wrapping_sub(m.model_yaw) as f32*unit,a.pitch as f32*unit));
        Self::new(yaw,pitch,m.fludd.as_ref().map_or(NozzleKind::Spray,|f|f.nozzle),&m.params)
    }
    pub fn direction(self)->Vec3 {Vec3::new(self.yaw.sin()*self.pitch.cos(),self.pitch.sin(),self.yaw.cos()*self.pitch.cos())}
}

impl YoshiSkin {
    pub fn load(dir:&Path,source:&Skeleton)->Result<Self,String> {
        let raw = std::fs::read(dir.join("yoshi/yoshi_model.bmd")).map_err(|e|e.to_string())?;
        let rig = Skeleton::from_bmd(&raw).ok_or("invalid Yoshi JNT1/INF1")?;
        if rig.len()!=38 { return Err(format!("expected Yoshi's 38 joints, got {}",rig.len())); }
        for &(target,name) in ALIASES {
            if rig.find(target).is_none() || source.find(name).is_none() {
                return Err(format!("missing retarget joint: {target} <- {name}"));
            }
        }
        let retarget = Retarget::new(source,&rig,ALIASES);
        // The neck's first joint is Yoshi's upper torso, mapped to Mario's chest.
        // Host attachment: keep the shared FLUDD in Mario's authored chest frame
        // (WaterGun.cpp::init/setBaseTRMtx), following Yoshi's posed torso position.
        let chest=rig.find("jnt_neck_1").ok_or("Yoshi torso joint missing")?;
        let mario_chest=source.find("jnt_chest").ok_or("Mario chest missing")?;
        let rotation=|m:Mat4|m.to_scale_rotation_translation().1;
        let chest_align=Mat4::from_quat(rotation(rig.bind_world()[chest]).inverse()
            *rotation(source.bind_world()[mario_chest]));
        let mouth=rig.find("null_tongue").ok_or("Yoshi mouth emitter missing")?;
        let head=rig.find("chn_head").ok_or("Yoshi head pivot missing")?;
        let jaw=rig.find("jnt_mouth").ok_or("Yoshi jaw missing")?;
        let eat=libsms::Clip::parse(&std::fs::read(dir.join("yoshi/yoshi_eat.bck")).map_err(|e|e.to_string())?)
            .ok_or("Yoshi eat/jaw animation missing")?;
        let mouth_frame=std::cell::Cell::new(rig.bind_world()[mouth]);
        eprintln!("Yoshi skin: yoshi.szs:yoshi_model.bmd, 38 joints; TYoshi::init (Player/Yoshi.cpp); Mario's 29-joint clips");
        Ok(Self{rig,retarget,chest,chest_align,mouth,head,jaw,eat,mouth_frame})
    }

    pub fn pose(&self,source:&[Mat4])->Vec<Mat4> { self.retarget.pose(&self.rig,source) }

    pub fn chest(&self,world:&[Mat4])->Mat4 { world[self.chest]*self.chest_align }

    pub fn pose_spray(&self,source:&[Mat4],route:SprayRoute,active:bool)->Vec<Mat4> {
        let mut world=self.pose(source);
        if route.mouth && active {
            // TYoshi::thinkUpper uses yoshi_eat over the mouth region. Keep the Mario
            // torso/arms while opening only the authored jaw at its held final pose.
            if let Some(local)=self.eat.sample(self.jaw,self.eat.frames) {
                world[self.jaw]=world[self.rig.parents[self.jaw].unwrap()]*local.matrix();
            }
            let forward=world[self.mouth].x_axis.truncate().normalize();
            let turn=Quat::from_rotation_arc(forward,route.direction());
            let pivot=world[self.head].w_axis.truncate();
            let delta=Mat4::from_translation(pivot)*Mat4::from_quat(turn)*Mat4::from_translation(-pivot);
            for j in 0..self.rig.len() {if self.rig.under(j,self.head) {world[j]=delta*world[j];}}
        }
        world
    }

    pub fn remember_mouth(&self,world:&[Mat4])->Mat4 {
        let m=world[self.mouth];self.mouth_frame.set(m);m
    }

    /// Route each simulation tick before feeding the shared water engine. Full carried
    /// velocity replaces vanilla's 0.125 share; emit_carried preserves it through caps.
    pub fn route_emit(&self,m:&Mario,w:&mut WaterEmit) {
        let route=SprayRoute::for_mario(m);
        if w.nozzle!=NozzleKind::Spray || !route.mouth {return}
        let rotation=Mat4::from_rotation_y(m.model_yaw as f32*std::f32::consts::PI/32768.);
        w.pos=m.pos+rotation.transform_point3(self.mouth_frame.get().w_axis.truncate());
        w.dir=rotation.transform_vector3(route.direction());
        w.inherit=m.vel;
        w.nozzle=NozzleKind::Yoshi;
        let n=m.params.nozzle(NozzleKind::Yoshi);
        let amount=m.fludd.as_ref().map_or(1.,|f|f.spray_amount().clamp(0.,1.));
        w.power=n.emit_pow_min+(n.emit_pow-n.emit_pow_min)*amount;
        w.size=n.size_min+(n.size-n.size_min)*amount;
        w.size_tremble=n.size_tremble;w.pow_tremble=n.pow_tremble;
        w.dir_tremble=n.dir_tremble;w.particle_type=n.particle_type;
        w.hit_radius=n.hit_radius;w.hit_height=n.hit_height;w.spin=false;
    }

    pub fn fludd_look(&self,m:&Mario,route:SprayRoute)->Option<libsms::FluddLook> {
        let mut look=m.fludd_look()?;
        if route.mouth {
            look.body_clip=libsms::fludd_anim::body_clip::HOUSE;
            look.body_frame=m.anims.get(look.body_clip).frames;
            look.nozzle_clip=Some("normal_wg_house");
            look.nozzle_frame=m.anims.get("normal_wg_house").frames;
            look.gun_angle=0.;look.aimed=false;look.aim_turn=0;
        }
        Some(look)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libsms_assets::{Bmd,Pose};
    #[test]
    fn spray_route_uses_bounded_front_head_aim_and_fludd_for_back_above_or_other_nozzles() {
        let p=Params::default();
        let route=|y:f32,pitch:f32,n|SprayRoute::new(y.to_radians(),pitch.to_radians(),n,&p);
        assert!(route(0.,0.,NozzleKind::Spray).mouth);
        assert!(route(-40.,20.,NozzleKind::Spray).mouth);
        assert!(route(40.,-20.,NozzleKind::Spray).mouth);
        assert!(!route(180.,0.,NozzleKind::Spray).mouth);
        assert!(!route(0.,65.,NozzleKind::Spray).mouth);
        assert!(!route(70.,0.,NozzleKind::Spray).mouth);
        assert!(!route(0.,0.,NozzleKind::Hover).mouth);
        assert!(!route(0.,0.,NozzleKind::Rocket).mouth);
        let clamped=route(170.,90.,NozzleKind::Spray);
        assert!(clamped.yaw.abs()<=60f32.to_radians()+1e-5);
        assert!(clamped.pitch<=45f32.to_radians()+1e-5);
    }
    #[test]
    fn mario_motion_keeps_yoshi_lengths_and_skins_finite_vertices() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets");
        if !dir.join("yoshi/yoshi_model.bmd").is_file() { eprintln!("SKIP: Yoshi disc cache absent"); return; }
        let source = Skeleton::from_bmd(&std::fs::read(dir.join("bmd/ma_mdl1.bmd")).unwrap()).unwrap();
        let skin = YoshiSkin::load(&dir,&source).unwrap();
        let bmd = Bmd::parse(&std::fs::read(dir.join("yoshi/yoshi_model.bmd")).unwrap()).unwrap();
        let batches = bmd.batches().unwrap();
        let bind = skin.rig.bind_world();
        for (a,b) in skin.pose(&source.bind_world()).iter().zip(&bind) { assert!(a.abs_diff_eq(*b,0.001)); }
        for name in ["wait","run1","run2","jump","dive_wait","hipat","hipsr"] {
            let c = libsms::Clip::parse(&std::fs::read(dir.join(format!("bck/ma_{name}.bck"))).unwrap()).unwrap();
            for frame in [0.,c.frames*0.25,c.frames*0.5,c.frames*0.75] {
                let world = skin.pose(&source.pose(&c,frame));
                let (scale,rotation,position)=skin.chest(&world).to_scale_rotation_translation();
                assert!(scale.abs_diff_eq(glam::Vec3::ONE,0.001),"FLUDD mount must not resize the pack");
                assert!(rotation.is_finite() && position.is_finite(),"invalid FLUDD mount {name}:{frame}");
                for (j,parent) in skin.rig.parents.iter().enumerate() {
                    let Some(p)=*parent else { continue };
                    let length=|w:&[Mat4]|w[j].w_axis.truncate().distance(w[p].w_axis.truncate());
                    assert!((length(&world)-length(&bind)).abs()<0.01,"{name}:{frame} bone {j}");
                }
                let pose = Pose::new(&bmd,Mat4::IDENTITY,&world);
                for v in batches.iter().flat_map(|b|&b.vertices) {
                    let (p,n)=pose.skin(v);
                    assert!(p.is_finite() && n.is_finite() && p.length()<2000.,"{name}:{frame}");
                }
                for (yaw,pitch) in [(-40f32,20f32),(0.,0.),(40.,-20.)] {
                    let route=SprayRoute::new(yaw.to_radians(),pitch.to_radians(),NozzleKind::Spray,&Params::default());
                    let aimed=skin.pose_spray(&source.pose(&c,frame),route,true);
                    let mouth=skin.remember_mouth(&aimed);
                    assert!(mouth.x_axis.truncate().normalize().dot(route.direction())>0.999);
                    for j in 0..skin.rig.len() {
                        if !skin.rig.under(j,skin.head) {assert!(aimed[j].abs_diff_eq(world[j],0.001));}
                    }
                }
            }
        }
    }
}
