use crate::float::DumbFloat16;
use crate::types::{Icon, ScreenTheme, UploadChannel};

pub trait Arg {
    const SIZE: usize;
    fn to_bytes(&self) -> Vec<u8>;
}

impl Arg for u8 {
    const SIZE: usize = 1;
    fn to_bytes(&self) -> Vec<u8> {
        vec![*self]
    }
}

impl Arg for u32 {
    const SIZE: usize = 4;
    fn to_bytes(&self) -> Vec<u8> {
        self.to_be_bytes().to_vec()
    }
}

macro_rules! impl_command_abi {
    [$(
        $( #[doc = $( $doc:tt )* ] )*
        fn $name:ident ( $([ $( $hardcode:expr ),* ]$(,)?)? $( $arg:ident: $type:tt ),* );
    )+] => {
        $(
            $(#[doc = concat!("Construct a payload for ", $($doc)*)])*
            #[allow(unused_mut, unused_variables, unused_assignments)]
            pub fn $name( $( $arg: $type ),* ) -> [u8; 33] {
                let len = const { 0 $($( + $hardcode - $hardcode + 1 )*)? $( + $type::SIZE )* };
                let mut buf = [0u8; 33];
                buf[0] = 0x0;
                buf[1] = 88;
                buf[2] = len as u8;
                let mut cur = 3;
                $($(
                    buf[cur] = $hardcode;
                    cur += 1;
                )*)?
                $(
                    let start = cur;
                    cur += $type::SIZE;
                    buf[start..cur].copy_from_slice(&$arg.to_bytes());
                )*
                buf
            }
        )*
    };
}

impl_command_abi![
    /* SCREEN POSITION */

    /// resetting screen back to meletrix logo
    fn reset_screen([165, 1, 255]);

    /// set the screen theme
    fn screen_theme([165, 1, 255], theme: ScreenTheme);

    /// moving the screen up one position
    fn screen_up([165, 0, 34]);

    /// moving the screen down one position
    fn screen_down([165, 0, 33]);

    /// switching the screen to the next page
    fn screen_switch([165, 0, 32]);

    /* MEDIA COMMANDS */

    /// deleting the currently uploaded image and reset back to the chrome dino
    fn delete_image([165, 2, 224]);

    /// deleting the currently uploaded gif and reset back to nyan cat
    fn delete_gif([165, 2, 225]);

    /// signaling the start of an upload
    fn upload_start([165, 2, 240], channel: UploadChannel);

    /// signaling the length of an upload
    fn upload_length([165, 2, 208], len: u32);

    /// signaling the end of an upload
    fn upload_end([165, 2, 241, 1]);

    /* SETTER COMMANDS */

    /// setting the system clock
    fn set_time([165, 1, 16], year: u8, month: u8, day: u8, hour: u8, minute: u8, second: u8);

    /// setting the weather icon and current/min/max temperatures
    fn set_weather([165, 1, 32], icon: Icon, current: u8, low: u8, high: u8);

    /// setting the cpu/gpu temp and download rate
    fn set_system_info([165, 1, 64], cpu_temp: u8, gpu_temp: u8, download: DumbFloat16);
];

/* GETTER COMMANDS */

/// Construct a payload for getting the abi version of the keyboard
pub const fn get_version() -> [u8; 33] {
    let mut buf = [0u8; 33];
    buf[1] = 1;
    buf
}
