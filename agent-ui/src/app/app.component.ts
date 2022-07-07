import { Component, OnInit } from '@angular/core';

@Component({
  selector: 'app-root',
  templateUrl: './app.component.html',
  styleUrls: ['./app.component.scss']
})
export class AppComponent implements OnInit {
  public userToken: string;
  public  proxyServerAddresses: string;
  public enableCompressing: boolean;
  constructor(){
    this.userToken="";
    this.proxyServerAddresses="";
    this.enableCompressing=false;
  }
  ngOnInit(): void {
    throw new Error('Method not implemented.');
  }
}
