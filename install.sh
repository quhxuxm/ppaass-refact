#Prepare base env
export RUSTUP_INIT_SKIP_PATH_CHECK=yes

sudo apt update
sudo apt upgrade -y
sudo apt install gcc -y
sudo apt install libfontconfig -y
sudo apt install libfontconfig1-dev -y
sudo iptables -A INPUT -p tcp --dport 8080 -j ACCEPT
sudo iptables -A INPUT -p tcp --dport 80 -j ACCEPT
#sudo mkdir /opt/java
#sudo apt install openjdk-17-jdk -y
#sudo apt install maven -y
sudo apt install unzip -y
sudo apt install git -y

sudo curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
# sudo wget https://download.java.net/java/GA/jdk15.0.2/0d1cfde4252546c6931946de8db48ee2/7/GPL/openjdk-15.0.2_linux-x64_bin.tar.gz
# sudo tar -zxf openjdk-15.0.2_linux-x64_bin.tar.gz -C /opt/java/
# sudo update-alternatives --install /usr/bin/java java /opt/java/jdk-15.0.2/bin/java 100
# sudo update-alternatives --config java
source $HOME/.cargo/env
rustup update
#Create swap file
#sudo swapoff /swapfile
#sudo fallocate -l 4G /swapfile
#sudo chmod 600 /swapfile
#sudo mkswap /swapfile
#sudo swapon /swapfile
#sudo free -h
#echo '/swapfile none swap sw 0 0' | sudo tee -a /etc/fstab

# Start install ppaass
sudo ps -ef | grep ppaass-proxy | grep -v grep | awk '{print $2}' | xargs sudo kill

sudo rm -rf /ppaass/build
sudo rm -rf /ppaass/sourcecode
# Build
sudo mkdir /ppaass
sudo mkdir /ppaass/sourcecode
sudo mkdir /ppaass/build

# Pull ppaass-rust
cd /ppaass/sourcecode
sudo git clone https://github.com/quhxuxm/ppaass-refact.git ppaass
sudo chmod 777 ppaass
cd /ppaass/sourcecode/ppaass
sudo git pull
export RUST_BACKTRACE=1
cargo build --package proxy --release

# ps -ef | grep gradle | grep -v grep | awk '{print $2}' | xargs kill -9
sudo cp /ppaass/sourcecode/ppaass/ppaass-proxy.toml /ppaass/build
sudo cp /ppaass/sourcecode/ppaass/ppaass-proxy-log.toml /ppaass/build
sudo cp -r /ppaass/sourcecode/ppaass/rsa /ppaass/build
sudo cp /ppaass/sourcecode/ppaass/*.pem /ppaass/build
sudo cp /ppaass/sourcecode/ppaass/target/release/ppaass-proxy /ppaass/build
sudo cp /ppaass/sourcecode/ppaass/ppaass-proxy-start.sh /ppaass/build/

sudo chmod 777 /ppaass/build
cd /ppaass/build
ls -l

sudo chmod 777 ppaass-proxy
sudo chmod 777 *.sh

#Start with the low configuration by default
sudo nohup ./ppaass-proxy-start.sh >run.log 2>&1 &
#Start with the high configuration
#sudo nohup ./start-high.sh >run.log 2>&1 &

ps -ef|grep ppaass-proxy
