//! Explicit command tracking. Notifications are best-effort and never own
//! the child's process, output, privileges, or exit status.
use anyhow::{bail,Result};
use std::{collections::HashMap,ffi::{OsStr,OsString},io,os::unix::process::ExitStatusExt,process::{Command,ExitStatus},sync::mpsc,time::{Duration,Instant}};
use zbus::{blocking::{Connection,Proxy},zvariant::OwnedValue};
use crate::{NAME,PATH,HISTORY_PATH,HISTORY_INTERFACE};

struct Run<'a>{label:&'a str,command:&'a OsStr,args:&'a [OsString]}
fn parse(args:&[OsString])->Result<Run<'_>>{
    let [label,separator,command,rest @ ..]=args else{bail!("usage: atelier-notifications run LABEL -- COMMAND [ARGS...]");};
    let Some(label)=label.to_str()else{bail!("notification label must be valid UTF-8");};
    if label.trim().is_empty()||label.chars().count()>160||label.chars().any(char::is_control){
        bail!("notification label must contain 1–160 printable characters");
    }
    if separator!="--"||command.is_empty(){bail!("expected -- followed by an executable");}
    Ok(Run{label,command,args:rest})
}
#[derive(Clone,Copy)]
struct Outcome{code:i32,description:&'static str}
fn outcome(result:io::Result<ExitStatus>)->Outcome{
    match result{
        Ok(status) if status.success()=>Outcome{code:0,description:"Completed successfully"},
        Ok(status)=>match status.code(){
            Some(code)=>Outcome{code,description:"Command failed"},
            None=>Outcome{code:128+status.signal().unwrap_or(0),description:"Command terminated by a signal"},
        },
        Err(error) if error.kind()==io::ErrorKind::NotFound=>Outcome{code:127,description:"Executable not found"},
        Err(_)=>Outcome{code:126,description:"Could not execute command"},
    }
}
struct Completion{outcome:Outcome,elapsed:Duration}
fn publish(proxy:&Proxy<'_>,id:u32,label:&str,body:&str,icon:&str,urgency:u8,timeout:i32)->zbus::Result<u32>{
    let hints=HashMap::from([("urgency",OwnedValue::from(urgency))]);
    proxy.call("Notify",&("Atelier CLI",id,icon,label,body,Vec::<String>::new(),hints,timeout))
}
fn track(label:&str,finished:mpsc::Receiver<Completion>)->Result<()>{
    let connection=Connection::session()?;
    let bus=Proxy::new(&connection,"org.freedesktop.DBus","/org/freedesktop/DBus","org.freedesktop.DBus")?;
    // Pin both calls to the same unique owner. Never replace an unrelated ID
    // after daemon restart, and never send to a different notification service.
    let owner:String=bus.call("GetNameOwner",&(NAME,))?;
    let extension=Proxy::new(&connection,owner.as_str(),HISTORY_PATH,HISTORY_INTERFACE)?;
    let _:String=extension.call("List",&())?;
    let target=Proxy::new(&connection,owner.as_str(),PATH,NAME)?;
    // A finite running card avoids leaving persistent orphans if the wrapper
    // is interrupted. Long jobs still receive a completion card if available.
    let id=publish(&target,0,label,"Running…","system-run-symbolic",1,60000)?;
    let completion=finished.recv()?;
    let result=completion.outcome;
    let body=format!("{} · exit {} · {}s",result.description,result.code,completion.elapsed.as_secs());
    let (icon,urgency,timeout)=if result.code==0{("emblem-ok-symbolic",1,5000)}else{("dialog-error-symbolic",2,8000)};
    publish(&target,id,label,&body,icon,urgency,timeout)?;
    Ok(())
}

pub fn run(args:&[OsString])->Result<i32>{
    let run=parse(args)?;
    let label=run.label.to_owned();
    let (finish,finished)=mpsc::channel::<Completion>();
    let (done,ack)=mpsc::sync_channel(1);
    let tracker=std::thread::Builder::new().name("atelier-command-notification".into()).spawn(move||{
        let _=done.send(track(&label,finished).is_ok());
    });
    let start=Instant::now();
    // No implicit shell, environment rewriting, output capture or stdin
    // interception. OsString preserves non-UTF-8 command arguments.
    let result=Command::new(run.command).args(run.args).status();
    if let Err(error)=&result{eprintln!("atelier-notifications run: {error}");}
    let result=outcome(result);
    let _=finish.send(Completion{outcome:result,elapsed:start.elapsed()});
    // A broken or slow bus must not prevent the command from running or
    // indefinitely delay the wrapper's exit. Main exits the process after
    // this bounded best-effort wait, even if the tracking thread is stalled.
    if tracker.is_err()||ack.recv_timeout(Duration::from_secs(2))!=Ok(true){
        eprintln!("atelier-notifications: command notification unavailable");
    }
    Ok(result.code)
}

#[cfg(test)]
mod tests{
    use super::*;
    fn args(values:&[&str])->Vec<OsString>{values.iter().map(OsString::from).collect()}
    #[test]
    fn explicit_separator_and_label_are_required(){
        for invalid in [vec![],vec!["label","echo"],vec!["label","--"],vec!["","--","echo"],vec!["a\nb","--","echo"]]{
            assert!(parse(&args(&invalid)).is_err());
        }
        let values=args(&["Build","--","printf","%s","a; b"]);
        let run=parse(&values).unwrap();assert_eq!(run.label,"Build");assert_eq!(run.command,"printf");
        assert_eq!(run.args,&args(&["%s","a; b"]));
    }
    #[test]
    fn exit_codes_are_not_notification_results(){
        for code in [0,1,7,125]{
            assert_eq!(outcome(Ok(ExitStatus::from_raw(code<<8))).code,code);
        }
        assert_eq!(outcome(Ok(ExitStatus::from_raw(15))).code,143);
        assert_eq!(outcome(Err(io::Error::from(io::ErrorKind::NotFound))).code,127);
        assert_eq!(outcome(Err(io::Error::from(io::ErrorKind::PermissionDenied))).code,126);
    }
}
