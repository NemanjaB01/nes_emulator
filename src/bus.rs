use crate::apu::Apu;
use crate::audio::SharedRingBuffer;
use crate::cartridge::Rom;
use crate::cpu::Mem;
use crate::joypad::Joypad;
use crate::ppu::NesPPU;
use crate::ppu::PPU;
use std::f32::consts::PI;

//  _______________ $10000  _______________
// | PRG-ROM       |       |               |
// | Upper Bank    |       |               |
// |_ _ _ _ _ _ _ _| $C000 | PRG-ROM       |
// | PRG-ROM       |       |               |
// | Lower Bank    |       |               |
// |_______________| $8000 |_______________|
// | SRAM          |       | SRAM          |
// |_______________| $6000 |_______________|
// | Expansion ROM |       | Expansion ROM |
// |_______________| $4020 |_______________|
// | I/O Registers |       |               |
// |_ _ _ _ _ _ _ _| $4000 |               |
// | Mirrors       |       | I/O Registers |
// | $2000-$2007   |       |               |
// |_ _ _ _ _ _ _ _| $2008 |               |
// | I/O Registers |       |               |
// |_______________| $2000 |_______________|
// | Mirrors       |       |               |
// | $0000-$07FF   |       |               |
// |_ _ _ _ _ _ _ _| $0800 |               |
// | RAM           |       | RAM           |
// |_ _ _ _ _ _ _ _| $0200 |               |
// | Stack         |       |               |
// |_ _ _ _ _ _ _ _| $0100 |               |
// | Zero Page     |       |               |
// |_______________| $0000 |_______________|
const RAM: u16 = 0x0000;
const RAM_MIRRORS_END: u16 = 0x1FFF;
const PPU_REGISTERS: u16 = 0x2000;
const PPU_REGISTERS_MIRRORS_END: u16 = 0x3FFF;
const CPU_FREQUENCY: f64 = 1_789_773.0;
const AUDIO_SAMPLE_RATE: u32 = 44_100;
const AUDIO_BUFFER_SAMPLES: usize = 512;
const AUDIO_OUTPUT_GAIN: f32 = 1.0;
const AUDIO_HP1_CUTOFF: f32 = 90.0;
const AUDIO_HP2_CUTOFF: f32 = 440.0;
const AUDIO_LP_CUTOFF: f32 = 14_000.0;

struct HighPassFilter {
    alpha: f32,
    prev_input: f32,
    prev_output: f32,
}

impl HighPassFilter {
    fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let rc = 1.0 / (2.0 * PI * cutoff_hz);
        let dt = 1.0 / sample_rate;
        let alpha = rc / (rc + dt);
        Self {
            alpha,
            prev_input: 0.0,
            prev_output: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.alpha * (self.prev_output + input - self.prev_input);
        self.prev_input = input;
        self.prev_output = output;
        output
    }
}

struct LowPassFilter {
    alpha: f32,
    prev_output: f32,
}

impl LowPassFilter {
    fn new(cutoff_hz: f32, sample_rate: f32) -> Self {
        let rc = 1.0 / (2.0 * PI * cutoff_hz);
        let dt = 1.0 / sample_rate;
        let alpha = dt / (rc + dt);
        Self {
            alpha,
            prev_output: 0.0,
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let output = self.prev_output + self.alpha * (input - self.prev_output);
        self.prev_output = output;
        output
    }
}

struct AudioFilterChain {
    hp1: HighPassFilter,
    hp2: HighPassFilter,
    lp: LowPassFilter,
}

impl AudioFilterChain {
    fn new(sample_rate: f32) -> Self {
        let sample_rate = sample_rate.max(1.0);
        Self {
            hp1: HighPassFilter::new(AUDIO_HP1_CUTOFF, sample_rate),
            hp2: HighPassFilter::new(AUDIO_HP2_CUTOFF, sample_rate),
            lp: LowPassFilter::new(AUDIO_LP_CUTOFF, sample_rate),
        }
    }

    fn process(&mut self, input: f32) -> f32 {
        let high1 = self.hp1.process(input);
        let high2 = self.hp2.process(high1);
        self.lp.process(high2)
    }
}

pub struct Bus<'call> {
    cpu_vram: [u8; 2048],
    prg_rom: Vec<u8>,
    ppu: NesPPU,
    apu: Apu,

    cycles: usize,
    gameloop_callback: Box<dyn FnMut(&NesPPU, &mut Joypad) + 'call>,
    joypad1: Joypad,
    audio_output: Option<SharedRingBuffer>,
    audio_sample_timer: f64,
    audio_sample_period: f64,
    audio_sample_accum: f32,
    audio_sample_count: u32,
    audio_filters: Option<AudioFilterChain>,
    audio_buffer: Vec<f32>,
}

impl<'a> Bus<'a> {
    pub fn new<'call, F>(rom: Rom, gameloop_callback: F) -> Bus<'call>
    where
        F: FnMut(&NesPPU, &mut Joypad) + 'call,
    {
        let ppu = NesPPU::new(rom.chr_rom, rom.screen_mirroring);

        Bus {
            cpu_vram: [0; 2048],
            prg_rom: rom.prg_rom,
            ppu: ppu,
            apu: Apu::new(),
            cycles: 0,
            gameloop_callback: Box::from(gameloop_callback),
            joypad1: Joypad::new(),
            audio_output: None,
            audio_sample_timer: 0.0,
            audio_sample_period: CPU_FREQUENCY / AUDIO_SAMPLE_RATE as f64,
            audio_sample_accum: 0.0,
            audio_sample_count: 0,
            audio_filters: None,
            audio_buffer: Vec::with_capacity(AUDIO_BUFFER_SAMPLES),
        }
    }

    pub fn attach_audio_buffer(&mut self, audio_output: SharedRingBuffer, sample_rate: u32) {
        let sample_rate = if sample_rate == 0 {
            AUDIO_SAMPLE_RATE
        } else {
            sample_rate
        };
        if let Ok(mut buffer) = audio_output.lock() {
            buffer.clear();
        }
        self.audio_output = Some(audio_output);
        self.audio_sample_timer = 0.0;
        self.audio_sample_period = CPU_FREQUENCY / sample_rate as f64;
        self.audio_sample_accum = 0.0;
        self.audio_sample_count = 0;
        self.audio_filters = Some(AudioFilterChain::new(sample_rate as f32));
        self.audio_buffer.clear();
    }

    fn read_prg_rom(&self, mut addr: u16) -> u8 {
        addr -= 0x8000;
        if self.prg_rom.len() == 0x4000 && addr >= 0x4000 {
            //mirror if needed
            addr = addr % 0x4000;
        }
        self.prg_rom[addr as usize]
    }

    pub fn tick(&mut self, cycles: u8) {
        self.cycles += cycles as usize;

        let nmi_before = self.ppu.nmi_interrupt.is_some();
        self.ppu.tick(cycles *3);
        let nmi_after = self.ppu.nmi_interrupt.is_some();

        for _ in 0..cycles {
            self.apu.tick();
            if self.audio_output.is_some() {
                self.audio_sample_timer += 1.0;
                self.audio_sample_accum += self.apu.get_sample();
                self.audio_sample_count += 1;
                if self.audio_sample_timer >= self.audio_sample_period {
                    self.audio_sample_timer -= self.audio_sample_period;
                    if self.audio_sample_count > 0 {
                        let avg = self.audio_sample_accum / self.audio_sample_count as f32;
                        self.audio_sample_accum = 0.0;
                        self.audio_sample_count = 0;
                        let sample = self.filter_audio_sample(avg);
                        self.audio_buffer.push(sample);
                        if self.audio_buffer.len() >= AUDIO_BUFFER_SAMPLES {
                            self.queue_audio_samples();
                        }
                    }
                }
            }
        }
        
        if !nmi_before && nmi_after {
            (self.gameloop_callback)(&self.ppu, &mut self.joypad1);
        }
    }
    
    pub fn poll_nmi_status(&mut self) -> Option<u8> {
        self.ppu.poll_nmi_interrupt()
    }

    fn filter_audio_sample(&mut self, sample: f32) -> f32 {
        let mut output = sample;
        if let Some(filters) = self.audio_filters.as_mut() {
            output = filters.process(output);
        }
        output = output * AUDIO_OUTPUT_GAIN;
        output.max(-1.0).min(1.0)
    }

    fn queue_audio_samples(&mut self) {
        if self.audio_buffer.is_empty() {
            return;
        }
        if let Some(output) = self.audio_output.as_ref() {
            if let Ok(mut output) = output.lock() {
                output.push_slice(&self.audio_buffer);
            }
        }
        self.audio_buffer.clear();
    }
}

impl Mem for Bus<'_> {
    fn mem_read(&mut self, addr: u16) -> u8 {
        match addr {
            RAM..=RAM_MIRRORS_END => {
                let mirror_down_addr = addr & 0b00000111_11111111;
                self.cpu_vram[mirror_down_addr as usize]
            }
            0x2000 | 0x2001 | 0x2003 | 0x2005 | 0x2006 | 0x4014 => {
                // panic!("Attempt to read from write-only PPU address {:x}", addr);
                0
            }
            0x2002 => self.ppu.read_status(),
            0x2004 => self.ppu.read_oam_data(),
            0x2007 => self.ppu.read_data(),

            0x4000..=0x4013 | 0x4015 => self.apu.read_register(addr),

            0x4016 => {
                self.joypad1.read()
            }

            0x4017 => {
                // ignore joypad 2
                0
            }
            0x2008..=PPU_REGISTERS_MIRRORS_END => {
                let mirror_down_addr = addr & 0b00100000_00000111;
                self.mem_read(mirror_down_addr)
            }
            0x8000..=0xFFFF => self.read_prg_rom(addr),

            _ => {
                // println!("Ignoring mem access at {:x}", addr);
                0
            }
        }
    }

    fn mem_write(&mut self, addr: u16, data: u8) {
        match addr {
            RAM..=RAM_MIRRORS_END => {
                let mirror_down_addr = addr & 0b11111111111;
                self.cpu_vram[mirror_down_addr as usize] = data;
            }
            0x2000 => {
                self.ppu.write_to_ctrl(data);
            }
            0x2001 => {
                self.ppu.write_to_mask(data);
            }

            0x2002 => panic!("attempt to write to PPU status register"),

            0x2003 => {
                self.ppu.write_to_oam_addr(data);
            }
            0x2004 => {
                self.ppu.write_to_oam_data(data);
            }
            0x2005 => {
                self.ppu.write_to_scroll(data);
            }

            0x2006 => {
                self.ppu.write_to_ppu_addr(data);
            }
            0x2007 => {
                self.ppu.write_to_data(data);
            }
            0x4000..=0x4013 | 0x4015 | 0x4017 => {
                self.apu.write_register(addr, data);
            }

            0x4016 => {
                self.joypad1.write(data);
            }

            // https://wiki.nesdev.com/w/index.php/PPU_programmer_reference#OAM_DMA_.28.244014.29_.3E_write
            0x4014 => {
                let mut buffer: [u8; 256] = [0; 256];
                let hi: u16 = (data as u16) << 8;
                for i in 0..256u16 {
                    buffer[i as usize] = self.mem_read(hi + i);
                }

                self.ppu.write_oam_dma(&buffer);

                // todo: handle this eventually
                // let add_cycles: u16 = if self.cycles % 2 == 1 { 514 } else { 513 };
                // self.tick(add_cycles); //todo this will cause weird effects as PPU will have 513/514 * 3 ticks
            }

            0x2008..=PPU_REGISTERS_MIRRORS_END => {
                let mirror_down_addr = addr & 0b00100000_00000111;
                self.mem_write(mirror_down_addr, data);
                // todo!("PPU is not supported yet");
            }
            0x8000..=0xFFFF => panic!("Attempt to write to Cartridge ROM space: {:x}", addr),

            _ => {
                println!("Ignoring mem write-access at {:x}", addr);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::cartridge::test;

    #[test]
    fn test_mem_read_write_to_ram() {
        let mut bus = Bus::new(test::test_rom(), |_ppu, _joypad| {});
        bus.mem_write(0x01, 0x55);
        assert_eq!(bus.mem_read(0x01), 0x55);
    }
}
