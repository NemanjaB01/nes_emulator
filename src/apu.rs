const DUTY_TABLE: [[u8; 8]; 4] = [
    // 12.5%
    [0, 1, 0, 0, 0, 0, 0, 0],
    // 25%
    [0, 1, 1, 0, 0, 0, 0, 0],
    // 50%
    [0, 1, 1, 1, 1, 0, 0, 0],
    // 75%
    [1, 0, 0, 1, 1, 1, 1, 1],
];

// Noise period table (NTSC), in APU cycles
const NOISE_PERIOD_TABLE: [u16; 16] = [
    4, 8, 16, 32, 64, 96, 128, 160, 202, 254, 380, 508, 762, 1016, 2034, 4068,
];

pub struct PulseChannel {
    pub enabled: bool,
    pub duty: u8,
    pub volume: u8,
    pub timer: u16,

    pub timer_counter: u16,
    pub step: u8,
}

impl PulseChannel {
    pub fn new() -> Self {
        Self {
            enabled: false,
            duty: 0,
            volume: 0,
            timer: 0,
            timer_counter: 1,
            step: 0,
        }
    }
}

impl PulseChannel {
    pub fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x4000 => {
                // bits 6–7 = duty
                self.duty = (value >> 6) & 0b11;

                // bits 0–3 = volume
                self.volume = value & 0b1111;
            }
            0x4002 => {
                // timer low 8 bits
                self.timer = (self.timer & 0xFF00) | value as u16;
            }
            0x4003 => {
                // timer high 8 bits
                self.timer = (self.timer & 0x00FF) | (((value & 0x07) as u16) << 8);
                self.step = 0; 
                self.enabled = true; 
            }
            _ => {}
        }
    }

    pub fn tick(&mut self) {
        if !self.enabled {
            return;
        }

        if self.timer_counter == 0 {
            self.timer_counter = (self.timer + 1) * 2;
            self.step = (self.step + 1) % 8;
        } else {
            self.timer_counter -= 1;
        }
    }

    pub fn sample(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }

        let bit = DUTY_TABLE[self.duty as usize][self.step as usize];
        bit as f32 * self.volume as f32
    }
}

pub struct NoiseChannel {
    pub enabled: bool,
    pub volume: u8,
    pub timer: u16,
    pub timer_counter: u16,
    pub mode: bool,
    pub period_index: u8,

    // 15-bit shift register
    pub shift_register: u16,
}

impl NoiseChannel {
    pub fn new() -> Self {
        Self {
            enabled: false,
            volume: 0,
            timer: 0,
            timer_counter: 0,
            mode: false,
            period_index: 0,
            shift_register: 1, 
        }
    }
}

impl NoiseChannel {
    pub fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            // volume
            0x400C => {
                self.volume = value & 0b1111;
            }

            // timer
            0x400E => {
                self.mode = (value & 0x80) != 0;
                self.period_index = value & 0x0F;
                self.timer = NOISE_PERIOD_TABLE[self.period_index as usize] * 2;
            }

            // enable / restart
            0x400F => {
                self.enabled = true;
            }

            _ => {}
        }
    }
}

impl NoiseChannel {
    pub fn tick(&mut self) {
        if !self.enabled {
            return;
        }

        if self.timer_counter == 0 {
            self.timer_counter = self.timer;

            // XOR bit 0 with bit 1 (or bit 6 in short mode)
            let bit0 = self.shift_register & 1;
            let tap = if self.mode { 6 } else { 1 };
            let bit1 = (self.shift_register >> tap) & 1;
            let feedback = bit0 ^ bit1;

            // shift right
            self.shift_register >>= 1;

            // insert feedback at bit 14
            self.shift_register |= feedback << 14;
        } else {
            self.timer_counter -= 1;
        }
    }
}

impl NoiseChannel {
    pub fn sample(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }

        let bit = self.shift_register & 1;
        bit as f32 * self.volume as f32
    }
}

// DMC rate table (NTSC)
const DMC_RATE_TABLE: [u16; 16] = [
    428, 380, 340, 320, 286, 254, 226, 214, 190, 160, 142, 128, 106, 84, 72, 54,
];

pub struct DmcChannel {
    pub enabled: bool,
    pub irq_enabled: bool,
    pub loop_flag: bool,
    pub rate_index: u8,
    pub timer: u16,
    pub timer_counter: u16,
    pub output_level: u8,
    pub sample_address: u16,
    pub sample_length: u16,
    pub bits_remaining: u8,
    pub shift_register: u8,
}

impl DmcChannel {
    pub fn new() -> Self {
        Self {
            enabled: false,
            irq_enabled: false,
            loop_flag: false,
            rate_index: 0,
            timer: DMC_RATE_TABLE[0],
            timer_counter: 0,
            output_level: 0,
            sample_address: 0,
            sample_length: 0,
            bits_remaining: 0,
            shift_register: 0,
        }
    }

    pub fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x4010 => {
                self.irq_enabled = (value & 0x80) != 0;
                self.loop_flag = (value & 0x40) != 0;
                self.rate_index = value & 0x0F;
                self.timer = DMC_RATE_TABLE[self.rate_index as usize];
            }
            0x4011 => {
               
                self.output_level = value & 0x7F;
            }
            0x4012 => {
                
                self.sample_address = 0xC000 + ((value as u16) * 64);
            }
            0x4013 => {
            
                self.sample_length = ((value as u16) * 16) + 1;
            }
            _ => {}
        }
    }

    pub fn tick(&mut self) {
        if !self.enabled {
            return;
        }

        if self.timer_counter == 0 {
            self.timer_counter = self.timer;

            if self.bits_remaining > 0 {

                if (self.shift_register & 1) != 0 {
                    if self.output_level <= 125 {
                        self.output_level += 2;
                    }
                } else {
                    if self.output_level >= 2 {
                        self.output_level -= 2;
                    }
                }
                self.shift_register >>= 1;
                self.bits_remaining -= 1;
            }
        } else {
            self.timer_counter -= 1;
        }
    }

    pub fn sample(&self) -> f32 {
        if !self.enabled {
            return 0.0;
        }

        self.output_level as f32
    }
}

// triangle wave sequence 
const TRIANGLE_TABLE: [u8; 32] = [
    15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
    13, 14, 15,
];

pub struct TriangleChannel {
    pub enabled: bool,
    pub timer: u16,
    pub timer_counter: u16,
    pub step: u8,
    pub linear_counter: u8,
    pub linear_counter_reload: u8,
    pub control_flag: bool,
}

impl TriangleChannel {
    pub fn new() -> Self {
        Self {
            enabled: false,
            timer: 0,
            timer_counter: 0,
            step: 0,
            linear_counter: 0,
            linear_counter_reload: 0,
            control_flag: false,
        }
    }

    pub fn cpu_write(&mut self, addr: u16, value: u8) {
        match addr {
            0x4008 => {

                self.linear_counter_reload = value & 0x7F;
                self.control_flag = (value & 0x80) != 0;
            }
            0x400A => {

                self.timer = (self.timer & 0xFF00) | value as u16;
            }
            0x400B => {

                self.timer = (self.timer & 0x00FF) | (((value & 0x07) as u16) << 8);
                self.linear_counter = self.linear_counter_reload;
                self.enabled = true;
            }
            _ => {}
        }
    }

    pub fn tick(&mut self) {
        if !self.enabled || self.linear_counter == 0 {
            return;
        }

        if self.timer_counter == 0 {
            self.timer_counter = self.timer + 1;
            self.step = (self.step + 1) % 32;
        } else {
            self.timer_counter -= 1;
        }
    }

    pub fn sample(&self) -> f32 {
        if !self.enabled || self.linear_counter == 0 {
            return 0.0;
        }

        TRIANGLE_TABLE[self.step as usize] as f32
    }
}

pub struct Apu {
    pub pulse1: PulseChannel,
    pub pulse2: PulseChannel,
    pub triangle: TriangleChannel,
    pub noise: NoiseChannel,
    pub dmc: DmcChannel,
    pub frame_counter: u32,
}

impl Apu {
    pub fn new() -> Self {
        Self {
            pulse1: PulseChannel::new(),
            pulse2: PulseChannel::new(),
            triangle: TriangleChannel::new(),
            noise: NoiseChannel::new(),
            dmc: DmcChannel::new(),
            frame_counter: 0,
        }
    }

    /// write to APU register
    pub fn write_register(&mut self, addr: u16, val: u8) {
        match addr {

            0x4000..=0x4003 => {
                self.pulse1.cpu_write(addr, val);
            }

            0x4004..=0x4007 => {

                self.pulse2.cpu_write(addr - 4, val);
            }

            0x4008..=0x400B => {
                self.triangle.cpu_write(addr, val);
            }

            0x400C..=0x400F => {
                self.noise.cpu_write(addr, val);
            }

            0x4010..=0x4013 => {
                self.dmc.cpu_write(addr, val);
            }

            0x4015 => {
                self.pulse1.enabled = (val & 0x01) != 0;
                self.pulse2.enabled = (val & 0x02) != 0;
                self.triangle.enabled = (val & 0x04) != 0;
                self.noise.enabled = (val & 0x08) != 0;
                self.dmc.enabled = (val & 0x10) != 0;
            }

            0x4017 => {
               // not needed now
            }
            _ => {}
        }
    }

    
    pub fn read_register(&mut self, addr: u16) -> u8 {
        match addr {
            
            0x4015 => {
                let mut status = 0u8;
                if self.pulse1.enabled {
                    status |= 0x01;
                }
                if self.pulse2.enabled {
                    status |= 0x02;
                }
                if self.triangle.enabled {
                    status |= 0x04;
                }
                if self.noise.enabled {
                    status |= 0x08;
                }
                if self.dmc.enabled {
                    status |= 0x10;
                }
                
                status
            }
            _ => 0,
        }
    }

    
    pub fn tick(&mut self) {
       
        self.pulse1.tick();
        self.pulse2.tick();
        self.triangle.tick();
        self.noise.tick();
        self.dmc.tick();
        self.frame_counter += 1;
    }


    pub fn get_sample(&self) -> f32 {

        let pulse_sum = self.pulse1.sample() + self.pulse2.sample();
        let pulse_out = if pulse_sum == 0.0 {
            0.0
        } else {
            95.88_f32 / (8128.0_f32 / pulse_sum + 100.0_f32)
        };

        let tnd_sum = self.triangle.sample() / 8227.0_f32
            + self.noise.sample() / 12241.0_f32
            + self.dmc.sample() / 22638.0_f32;
        let tnd_out = if tnd_sum == 0.0 {
            0.0
        } else {
            159.79_f32 / (1.0_f32 / tnd_sum + 100.0_f32)
        };

        (pulse_out + tnd_out).max(0.0).min(1.0)
    }
}
