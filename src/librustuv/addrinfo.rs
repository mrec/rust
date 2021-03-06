// Copyright 2013 The Rust Project Developers. See the COPYRIGHT
// file at the top-level directory of this distribution and at
// http://rust-lang.org/COPYRIGHT.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use libc::c_int;
use libc;
use std::mem;
use std::ptr::null;
use std::rt::task::BlockedTask;
use std::rt::rtio;

use net;
use super::{Loop, UvError, Request, wait_until_woken_after, wakeup};
use uvll;

struct Addrinfo {
    handle: *libc::addrinfo,
}

struct Ctx {
    slot: Option<BlockedTask>,
    status: c_int,
    addrinfo: Option<Addrinfo>,
}

pub struct GetAddrInfoRequest;

impl GetAddrInfoRequest {
    pub fn run(loop_: &Loop, node: Option<&str>, service: Option<&str>,
               hints: Option<rtio::AddrinfoHint>)
        -> Result<Vec<rtio::AddrinfoInfo>, UvError>
    {
        assert!(node.is_some() || service.is_some());
        let (_c_node, c_node_ptr) = match node {
            Some(n) => {
                let c_node = n.to_c_str();
                let c_node_ptr = c_node.with_ref(|r| r);
                (Some(c_node), c_node_ptr)
            }
            None => (None, null())
        };

        let (_c_service, c_service_ptr) = match service {
            Some(s) => {
                let c_service = s.to_c_str();
                let c_service_ptr = c_service.with_ref(|r| r);
                (Some(c_service), c_service_ptr)
            }
            None => (None, null())
        };

        let hint = hints.map(|hint| {
            libc::addrinfo {
                ai_flags: 0,
                ai_family: hint.family as c_int,
                ai_socktype: 0,
                ai_protocol: 0,
                ai_addrlen: 0,
                ai_canonname: null(),
                ai_addr: null(),
                ai_next: null(),
            }
        });
        let hint_ptr = hint.as_ref().map_or(null(), |x| x as *libc::addrinfo);
        let mut req = Request::new(uvll::UV_GETADDRINFO);

        return match unsafe {
            uvll::uv_getaddrinfo(loop_.handle, req.handle,
                                 getaddrinfo_cb, c_node_ptr, c_service_ptr,
                                 hint_ptr)
        } {
            0 => {
                req.defuse(); // uv callback now owns this request
                let mut cx = Ctx { slot: None, status: 0, addrinfo: None };

                wait_until_woken_after(&mut cx.slot, loop_, || {
                    req.set_data(&cx);
                });

                match cx.status {
                    0 => Ok(accum_addrinfo(cx.addrinfo.get_ref())),
                    n => Err(UvError(n))
                }
            }
            n => Err(UvError(n))
        };


        extern fn getaddrinfo_cb(req: *uvll::uv_getaddrinfo_t,
                                 status: c_int,
                                 res: *libc::addrinfo) {
            let req = Request::wrap(req);
            assert!(status != uvll::ECANCELED);
            let cx: &mut Ctx = unsafe { req.get_data() };
            cx.status = status;
            cx.addrinfo = Some(Addrinfo { handle: res });

            wakeup(&mut cx.slot);
        }
    }
}

impl Drop for Addrinfo {
    fn drop(&mut self) {
        unsafe { uvll::uv_freeaddrinfo(self.handle) }
    }
}

// Traverse the addrinfo linked list, producing a vector of Rust socket addresses
pub fn accum_addrinfo(addr: &Addrinfo) -> Vec<rtio::AddrinfoInfo> {
    unsafe {
        let mut addr = addr.handle;

        let mut addrs = Vec::new();
        loop {
            let rustaddr = net::sockaddr_to_addr(mem::transmute((*addr).ai_addr),
                                                 (*addr).ai_addrlen as uint);

            addrs.push(rtio::AddrinfoInfo {
                address: rustaddr,
                family: (*addr).ai_family as uint,
                socktype: 0,
                protocol: 0,
                flags: 0,
            });
            if (*addr).ai_next.is_not_null() {
                addr = (*addr).ai_next;
            } else {
                break;
            }
        }

        addrs
    }
}
