use gl::{
    GenQueries, GetInteger64v, GetQueryObjectiv, GetQueryObjectui64v, QueryCounter, QUERY_RESULT,
    QUERY_RESULT_AVAILABLE, TIMESTAMP,
};
use std::{
    ffi::CString,
    sync::atomic::{AtomicU8, Ordering},
};

use tracy_client_sys::{
    ___tracy_emit_gpu_context_name, ___tracy_emit_gpu_new_context, ___tracy_emit_gpu_time_serial,
    ___tracy_emit_gpu_zone_begin_serial, ___tracy_emit_gpu_zone_end_serial,
    ___tracy_gpu_context_name_data, ___tracy_gpu_new_context_data, ___tracy_gpu_time_data,
    ___tracy_gpu_zone_begin_data, ___tracy_gpu_zone_end_data, ___tracy_source_location_data,
};

use crate::profiling::{tracy_zone, GpuContextType, GpuCtx};

static CONTEXT_ID: AtomicU8 = AtomicU8::new(0);

struct GpuCtxOpenGL {
    id: u8,
    query: Vec<u32>,
    head: usize,
    tail: usize,
}

impl GpuCtxOpenGL {
    fn next_query_id(&mut self) -> usize {
        let query = self.head;
        self.head = (self.head + 1) % self.query.len();
        assert!(self.head != self.tail);
        query
    }
}

pub fn create_opengl_gpu_context(name: &str) -> Box<dyn GpuCtx> {
    let query_size = 64 * 1024;

    let mut ctx = Box::new(GpuCtxOpenGL {
        id: CONTEXT_ID.fetch_add(1, Ordering::Relaxed),
        query: Vec::with_capacity(query_size),
        head: 0,
        tail: 0,
    });
    unsafe {
        ctx.query.set_len(query_size);
        GenQueries(query_size as i32, ctx.query.as_mut_ptr());
    }
    let id = ctx.id;

    let mut timestamp: i64 = 0;
    unsafe {
        GetInteger64v(TIMESTAMP, &mut timestamp);
    }

    let ctxt_data = ___tracy_gpu_new_context_data {
        gpuTime: timestamp,
        period: 1.0,
        context: id,
        flags: 0,
        type_: GpuContextType::OpenGl as u8,
    };
    let namestring = CString::new(name).unwrap();
    let name_data = ___tracy_gpu_context_name_data {
        context: id,
        name: namestring.as_ptr(),
        len: name.len() as u16,
    };
    unsafe {
        ___tracy_emit_gpu_new_context(ctxt_data);
        ___tracy_emit_gpu_context_name(name_data);
    }

    ctx
}

impl GpuCtx for GpuCtxOpenGL {
    fn gpu_collect(&mut self) {
        tracy_zone!("collect gpu info");
        while self.tail != self.head {
            let mut available: i32 = 0;
            unsafe {
                GetQueryObjectiv(
                    self.query[self.tail],
                    QUERY_RESULT_AVAILABLE,
                    &mut available,
                );
            }
            if available <= 0 {
                break;
            }

            let mut time: u64 = 0;
            unsafe {
                GetQueryObjectui64v(self.query[self.tail], QUERY_RESULT, &mut time);
            }
            let time_data = ___tracy_gpu_time_data {
                gpuTime: time as i64,
                queryId: self.tail as u16,
                context: self.id,
            };
            unsafe {
                ___tracy_emit_gpu_time_serial(time_data);
            }
            self.tail = (self.tail + 1) % self.query.len();
        }
    }

    fn gpu_begin(&mut self, loc_data: &___tracy_source_location_data) -> i64 {
        let query = self.next_query_id();

        let gpu_data = ___tracy_gpu_zone_begin_data {
            srcloc: (loc_data as *const ___tracy_source_location_data) as u64,
            queryId: query as u16,
            context: self.id,
        };
        unsafe {
            QueryCounter(self.query[query], TIMESTAMP);
            ___tracy_emit_gpu_zone_begin_serial(gpu_data);
        }
        // Any positive id is fine here, since the opengl implementation does not use it
        1
    }

    fn gpu_end(&mut self, _query_id: i64) {
        let query = self.next_query_id();
        let gpu_data = ___tracy_gpu_zone_end_data {
            queryId: query as u16,
            context: self.id,
        };
        unsafe {
            QueryCounter(self.query[query], TIMESTAMP);
            ___tracy_emit_gpu_zone_end_serial(gpu_data);
        }
    }
}
