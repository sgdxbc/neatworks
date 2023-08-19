terraform {
  required_providers {
    aws = {
      source  = "hashicorp/aws"
      version = "~> 4.0"
    }
  }
}

provider "aws" {
  region = "ap-east-1"
}

resource "aws_vpc" "neat-pbft" {
  cidr_block           = "10.0.0.0/16"
  enable_dns_hostnames = true
}

resource "aws_subnet" "neat-pbft" {
  vpc_id                  = resource.aws_vpc.neat-pbft.id
  cidr_block              = "10.0.0.0/16"
  map_public_ip_on_launch = true
}

resource "aws_internet_gateway" "neat-pbft" {
  vpc_id = resource.aws_vpc.neat-pbft.id
}

resource "aws_route_table" "neat-pbft" {
  vpc_id = resource.aws_vpc.neat-pbft.id

  route {
    cidr_block = "0.0.0.0/0"
    gateway_id = resource.aws_internet_gateway.neat-pbft.id
  }
}

resource "aws_route_table_association" "_1" {
  route_table_id = resource.aws_route_table.neat-pbft.id
  subnet_id      = resource.aws_subnet.neat-pbft.id
}

resource "aws_security_group" "neat-pbft" {
  vpc_id = resource.aws_vpc.neat-pbft.id

  ingress {
    from_port        = 0
    to_port          = 0
    protocol         = "-1"
    cidr_blocks      = ["0.0.0.0/0"]
    ipv6_cidr_blocks = ["::/0"]
  }

  egress {
    from_port        = 0
    to_port          = 0
    protocol         = "-1"
    cidr_blocks      = ["0.0.0.0/0"]
    ipv6_cidr_blocks = ["::/0"]
  }
}

data "aws_ami" "ubuntu" {
  most_recent = true

  filter {
    name   = "name"
    values = ["ubuntu/images/hvm-ssd/ubuntu-jammy-22.04-amd64-server-*"]
  }

  filter {
    name   = "virtualization-type"
    values = ["hvm"]
  }

  owners = ["099720109477"] # Canonical
}

resource "aws_instance" "neat-pbft-replica" {
  count = 4 # TODO configurable

  ami                    = data.aws_ami.ubuntu.id
  instance_type          = "c5.xlarge"
  subnet_id              = resource.aws_subnet.neat-pbft.id
  vpc_security_group_ids = [resource.aws_security_group.neat-pbft.id]
  key_name               = "Ephemeral"
}

# output "instances" {
#   value = zipmap(resource.aws_instance.neat-pbft[*].public_ip, resource.aws_instance.neat-pbft[*].public_dns)
# }
